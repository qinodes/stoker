//! One-job process execution and durable cleanup.

use std::ffi::OsString;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::{broadcast, watch};

use crate::domain::{Job, JobState};
use crate::process::ProcessSpec;
use crate::{Store, StoreError};

use super::logs::{flush_log_events, watch_logs};
use super::{LogMessage, Scheduler};

pub(super) trait ExecutionStore: Send + Sync {
    fn get_job(&self, id: uuid::Uuid) -> Result<Job, StoreError>;
    fn set_running(&self, id: uuid::Uuid, pid: u32) -> Result<Job, StoreError>;
    fn finish(
        &self,
        id: uuid::Uuid,
        exit_code: Option<i32>,
        failure_detail: Option<&str>,
    ) -> Result<Job, StoreError>;
    fn clear_runtime(&self, id: uuid::Uuid) -> Result<Job, StoreError>;
    fn record_failure_detail(&self, id: uuid::Uuid, detail: &str) -> Result<Job, StoreError>;
}

impl ExecutionStore for Store {
    fn get_job(&self, id: uuid::Uuid) -> Result<Job, StoreError> {
        Store::get_job(self, id)
    }

    fn set_running(&self, id: uuid::Uuid, pid: u32) -> Result<Job, StoreError> {
        Store::set_running(self, id, pid)
    }

    fn finish(
        &self,
        id: uuid::Uuid,
        exit_code: Option<i32>,
        failure_detail: Option<&str>,
    ) -> Result<Job, StoreError> {
        Store::finish(self, id, exit_code, failure_detail)
    }

    fn clear_runtime(&self, id: uuid::Uuid) -> Result<Job, StoreError> {
        Store::clear_runtime(self, id)
    }

    fn record_failure_detail(&self, id: uuid::Uuid, detail: &str) -> Result<Job, StoreError> {
        Store::record_failure_detail(self, id, detail)
    }
}

impl Scheduler {
    pub(super) async fn execute(
        &self,
        job: Job,
        mut cancel: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        let run_dir = self.paths.runs.join(job.id.to_string());
        let stdout = run_dir.join("stdout.log");
        let stderr = run_dir.join("stderr.log");
        // Jobs intentionally run in the source directory recorded at add
        // time. Stoker does not inspect or manage that directory's contents.
        let cwd = job.cwd.clone();
        let mut sender = None;
        let result = async {
            // Keep setup inside the guarded path so a filesystem failure is
            // persisted as FAILED instead of escaping with STARTING claimed.
            tokio::fs::create_dir_all(&run_dir).await?;
            tokio::fs::write(&stdout, &[]).await?;
            tokio::fs::write(&stderr, &[]).await?;
            let (log_sender, _) = broadcast::channel(256);
            self.logs
                .lock()
                .map_err(|_| anyhow::anyhow!("scheduler log map is poisoned"))?
                .insert(job.id, log_sender.clone());
            sender = Some(log_sender.clone());
            let metadata = tokio::fs::metadata(&cwd)
                .await
                .with_context(|| format!("inspect job cwd {}", cwd.display()))?;
            if !metadata.is_dir() {
                anyhow::bail!("job cwd {} is not a directory", cwd.display());
            }
            let (program, args): (OsString, Vec<OsString>) =
                if let Some(command_line) = job.command_line.as_deref() {
                    #[cfg(unix)]
                    {
                        ("sh".into(), vec!["-c".into(), command_line.into()])
                    }
                    #[cfg(windows)]
                    {
                        ("cmd.exe".into(), vec!["/C".into(), command_line.into()])
                    }
                } else {
                    let (program, args) = job
                        .command
                        .split_first()
                        .ok_or_else(|| anyhow::anyhow!("job command is empty"))?;
                    (
                        program.clone().into(),
                        args.iter().cloned().map(Into::into).collect(),
                    )
                };
            let process = self
                .controller
                .spawn(ProcessSpec {
                    program,
                    args,
                    cwd,
                    stdout_log: stdout.clone(),
                    stderr_log: stderr.clone(),
                })
                .await
                .context("spawn job process")?;
            let process = process;
            let pid = process.pid();
            let process_cancel = tokio::sync::oneshot::channel();
            let mut process_cancel_tx = Some(process_cancel.0);
            let process_cancel_rx = process_cancel.1;
            let mut process_task = tokio::spawn(async move {
                process.wait_with_cancel(process_cancel_rx).await
            });
            let running = self.execution_store.set_running(job.id, pid);
            if let Err(error) = running {
                let _ = process_cancel_tx.take().map(|sender| sender.send(()));
                let _ = (&mut process_task).await;
                if self
                    .execution_store
                    .get_job(job.id)
                    .map(|current| current.state == JobState::Cancelling)
                    .unwrap_or(false)
                {
                    // Cancellation raced with the STARTING -> RUNNING
                    // transition. The normal terminal cleanup below will
                    // turn CANCELLING into CANCELLED.
                } else {
                    return Err(error).context(format!(
                        "record running job while job is {}",
                        self.execution_store
                            .get_job(job.id)
                            .map(|current| current.state.to_string())
                            .unwrap_or_else(|_| "unknown".into())
                    ));
                }
            }
            let watcher = tokio::spawn(watch_logs(
                stdout.clone(),
                stderr.clone(),
                log_sender.clone(),
            ));
            let status = if *cancel.borrow() {
                let _ = self.store.request_cancelling(job.id);
                let _ = process_cancel_tx.take().map(|sender| sender.send(()));
                (&mut process_task)
                    .await
                    .map_err(|error| anyhow::anyhow!("wait for cancelled job process: {error}"))?
                    .context("wait for cancelled job process")?
            } else {
                tokio::select! {
                    status = &mut process_task => status
                        .map_err(|error| anyhow::anyhow!("wait for job process: {error}"))?
                        .context("wait for job process")?,
                    changed = cancel.changed() => {
                        if changed.is_err() || !*cancel.borrow() {
                            return Err(anyhow::anyhow!("cancellation signal closed"));
                        }
                        if let Err(error) = self.store.request_cancelling(job.id) {
                            let current = self.store.get_job(job.id).context("inspect cancelling job")?;
                            if current.state != JobState::Cancelling {
                                return Err(error).context(format!(
                                    "mark job cancelling while job is {}",
                                    current.state
                                ));
                            }
                        }
                        let _ = process_cancel_tx.take().map(|sender| sender.send(()));
                        (&mut process_task)
                            .await
                            .map_err(|error| anyhow::anyhow!("wait for cancelled job process: {error}"))?
                            .context("wait for cancelled job process")?
                    }
                }
            };
            watcher.abort();
            let _ = watcher.await;
            flush_log_events(&stdout, &stderr, &log_sender).await;
            let code = status.code();
            self.execution_store
                .finish(job.id, code, None)
                .context("record job result")?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let mut diagnostics = Vec::new();
        if let Err(error) = result {
            let detail = format!("{error:#}");
            diagnostics.push(detail.clone());
            if let Err(finish_error) = self.execution_store.finish(job.id, None, Some(&detail)) {
                diagnostics.push(format!("record FAILED result: {finish_error}"));
            }
        }
        let mut cleanup_failed = false;
        let mut cleared = false;
        let mut clear_error = None;
        let mut job_removed = false;
        for _ in 0..3 {
            match self.execution_store.clear_runtime(job.id) {
                Ok(_) => {
                    cleared = true;
                    break;
                }
                Err(StoreError::NotFound { .. }) => {
                    // `stoker clean` may remove a terminal job after finish
                    // persisted it but before scheduler cleanup completed.
                    cleared = true;
                    job_removed = true;
                    break;
                }
                Err(error) => {
                    clear_error = Some(error.to_string());
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
        if !cleared {
            cleanup_failed = true;
            diagnostics.push(format!(
                "clear runtime fields: {}",
                clear_error.unwrap_or_else(|| "unknown error".into())
            ));
        }
        let mut diagnostics_persistence_error = None;
        if !diagnostics.is_empty() && !job_removed {
            let detail = diagnostics.join("; ");
            if let Err(error) = self.execution_store.record_failure_detail(job.id, &detail) {
                if !matches!(error, StoreError::NotFound { .. }) {
                    diagnostics_persistence_error = Some(format!(
                        "persist scheduler diagnostics for job {}: {error}; {detail}",
                        job.id
                    ));
                }
            } else {
                eprintln!("job {} scheduler diagnostics: {detail}", job.id);
            }
        }
        if let Some(sender) = sender {
            let _ = sender.send(LogMessage::End);
        }
        self.logs.lock().ok().map(|mut logs| logs.remove(&job.id));
        if let Some(error) = diagnostics_persistence_error {
            return Err(anyhow::anyhow!(error));
        }
        if cleanup_failed {
            return Err(anyhow::anyhow!(
                "cleanup incomplete for job {}; scheduler stopped with active slot held",
                job.id
            ));
        }
        Ok(())
    }
}
