//! Process execution for flow task attempts.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::oneshot;

use crate::process::{LogCapturePolicy, ProcessLaunchPolicy, ProcessSpec};
use crate::store::{FlowAttemptResult, FlowTaskExecution};

use super::Scheduler;

impl Scheduler {
    pub(super) async fn execute_flow_task(
        &self,
        execution: FlowTaskExecution,
    ) -> anyhow::Result<()> {
        let run_dir = self
            .paths
            .runs
            .join("flows")
            .join(execution.run_id.to_string())
            .join(&execution.task_id)
            .join(format!("attempt-{}", execution.attempt_number));
        let stdout = run_dir.join("stdout.log");
        let stderr = run_dir.join("stderr.log");
        let runtime_policy = self
            .store
            .runtime_policy()
            .context("read flow runtime policy")?;
        let log_policy = self.store.log_policy().context("read flow log policy")?;
        let setup = tokio::time::timeout(
            Duration::from_millis(runtime_policy.startup_timeout_ms),
            async {
                tokio::fs::create_dir_all(&run_dir).await?;
                tokio::fs::write(&stdout, &[]).await?;
                tokio::fs::write(&stderr, &[]).await?;
                let metadata = tokio::fs::metadata(PathBuf::from(&execution.task.cwd)).await?;
                if !metadata.is_dir() {
                    return Err(std::io::Error::other("flow task cwd is not a directory"));
                }
                Ok::<(), std::io::Error>(())
            },
        )
        .await;
        match setup {
            Err(error) => {
                self.finish_flow_failure(&execution, "STARTUP_TIMEOUT", error.to_string())
                    .await?;
                return Ok(());
            }
            Ok(Err(error)) => {
                self.finish_flow_failure(&execution, "STARTUP", error.to_string())
                    .await?;
                return Ok(());
            }
            Ok(Ok(())) => {}
        }

        let (program, args): (OsString, Vec<OsString>) = {
            #[cfg(unix)]
            {
                (
                    "sh".into(),
                    vec!["-c".into(), execution.task.command.clone().into()],
                )
            }
            #[cfg(windows)]
            {
                (
                    "cmd.exe".into(),
                    vec!["/C".into(), execution.task.command.clone().into()],
                )
            }
        };
        let process = match self
            .controller
            .spawn_with_policy(
                ProcessSpec {
                    program,
                    args,
                    cwd: PathBuf::from(&execution.task.cwd),
                    stdout_log: stdout,
                    stderr_log: stderr,
                },
                ProcessLaunchPolicy {
                    log_policy: LogCapturePolicy {
                        max_bytes_per_job: log_policy.max_bytes_per_job,
                        segment_bytes: log_policy.segment_bytes,
                    },
                    termination_grace: Duration::from_millis(runtime_policy.termination_grace_ms),
                },
            )
            .await
        {
            Ok(process) => process,
            Err(error) => {
                self.finish_flow_failure(&execution, "SPAWN", error.to_string())
                    .await?;
                return Ok(());
            }
        };
        self.store.mark_flow_attempt_running(execution.attempt_id)?;
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let mut cancel_tx = Some(cancel_tx);
        let mut wait_task = tokio::spawn(async move { process.wait_with_cancel(cancel_rx).await });
        let (status, forced) = if let Some(milliseconds) = runtime_policy.max_runtime_ms {
            let timeout = tokio::time::sleep(Duration::from_millis(milliseconds));
            tokio::pin!(timeout);
            let mut poll = tokio::time::interval(Duration::from_millis(50));
            loop {
                tokio::select! {
                    result = &mut wait_task => {
                        break (result.context("join flow process wait")?, None);
                    }
                    _ = poll.tick() => {
                        if self.store.flow_attempt_cancel_requested(execution.attempt_id)? {
                            if let Some(sender) = cancel_tx.take() { let _ = sender.send(()); }
                            break (wait_task.await.context("join cancelled flow process wait")?, Some("CANCELLED"));
                        }
                    }
                    _ = &mut timeout => {
                        if let Some(sender) = cancel_tx.take() { let _ = sender.send(()); }
                        break (wait_task.await.context("join timed-out flow process wait")?, Some("TIMEOUT"));
                    }
                }
            }
        } else {
            let mut poll = tokio::time::interval(Duration::from_millis(50));
            loop {
                tokio::select! {
                    result = &mut wait_task => {
                        break (result.context("join flow process wait")?, None);
                    }
                    _ = poll.tick() => {
                        if self.store.flow_attempt_cancel_requested(execution.attempt_id)? {
                            if let Some(sender) = cancel_tx.take() { let _ = sender.send(()); }
                            break (wait_task.await.context("join cancelled flow process wait")?, Some("CANCELLED"));
                        }
                    }
                }
            }
        };
        match (status, forced) {
            (Ok(_), Some("CANCELLED")) => {
                self.store.finish_flow_attempt(
                    execution.attempt_id,
                    FlowAttemptResult::Cancelled {
                        detail: "cancel requested".into(),
                    },
                )?;
            }
            (Ok(_), Some("TIMEOUT")) => {
                self.store.finish_flow_attempt(
                    execution.attempt_id,
                    FlowAttemptResult::Failed {
                        exit_code: None,
                        kind: "RUNTIME_TIMEOUT".into(),
                        detail: "flow task exceeded max runtime".into(),
                    },
                )?;
            }
            (Ok(_), Some(_)) => {
                self.store.finish_flow_attempt(
                    execution.attempt_id,
                    FlowAttemptResult::Failed {
                        exit_code: None,
                        kind: "RUNTIME_TIMEOUT".into(),
                        detail: "flow task was forcibly terminated".into(),
                    },
                )?;
            }
            (Ok(status), None) if status.success() => {
                self.store.finish_flow_attempt(
                    execution.attempt_id,
                    FlowAttemptResult::Succeeded {
                        exit_code: status.code().unwrap_or(0),
                    },
                )?;
            }
            (Ok(status), None) => {
                self.store.finish_flow_attempt(
                    execution.attempt_id,
                    FlowAttemptResult::Failed {
                        exit_code: status.code(),
                        kind: "EXIT".into(),
                        detail: format!(
                            "process exited with {}",
                            status
                                .code()
                                .map_or_else(|| "no exit code".into(), |code| code.to_string())
                        ),
                    },
                )?;
            }
            (Err(error), _) => {
                self.finish_flow_unknown(&execution, error.to_string())
                    .await?;
            }
        }
        Ok(())
    }

    async fn finish_flow_failure(
        &self,
        execution: &FlowTaskExecution,
        kind: &str,
        detail: String,
    ) -> anyhow::Result<()> {
        self.store.finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Failed {
                exit_code: None,
                kind: kind.into(),
                detail,
            },
        )?;
        Ok(())
    }

    async fn finish_flow_unknown(
        &self,
        execution: &FlowTaskExecution,
        detail: String,
    ) -> anyhow::Result<()> {
        self.store
            .finish_flow_attempt(execution.attempt_id, FlowAttemptResult::Lost { detail })?;
        Ok(())
    }
}
