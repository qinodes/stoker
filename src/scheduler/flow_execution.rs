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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use crate::config::{RuntimePolicy, StokerPaths};
    use crate::domain::flow::{ExecutionMode, ScheduleSpec};
    use crate::process::{ManagedProcess, ProcessController, ProcessSpec};
    use crate::store::FlowTaskInput;
    use async_trait::async_trait;
    use chrono::{Duration as ChronoDuration, Utc};
    use std::io;
    use std::path::Path;
    use std::process::ExitStatus;
    use std::sync::Arc;

    struct SpawnFailure;
    struct TimeoutController;
    struct TimeoutProcess;
    struct ExitController;
    struct ExitProcess;
    struct WaitFailureController;
    struct WaitFailureProcess;

    #[async_trait]
    impl ProcessController for SpawnFailure {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Err(io::Error::other("injected flow spawn failure"))
        }
    }

    #[async_trait]
    impl ProcessController for TimeoutController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(TimeoutProcess))
        }
    }

    #[async_trait]
    impl ProcessController for ExitController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(ExitProcess))
        }
    }

    #[async_trait]
    impl ProcessController for WaitFailureController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(WaitFailureProcess))
        }
    }

    #[async_trait]
    impl ManagedProcess for TimeoutProcess {
        fn pid(&self) -> u32 {
            42
        }

        async fn wait(self: Box<Self>) -> io::Result<ExitStatus> {
            Ok(successful_status())
        }

        async fn wait_with_cancel(
            self: Box<Self>,
            cancel: oneshot::Receiver<()>,
        ) -> io::Result<ExitStatus> {
            let _ = cancel.await;
            Ok(successful_status())
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ManagedProcess for ExitProcess {
        fn pid(&self) -> u32 {
            43
        }

        async fn wait(self: Box<Self>) -> io::Result<ExitStatus> {
            Ok(failed_status())
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ManagedProcess for WaitFailureProcess {
        fn pid(&self) -> u32 {
            44
        }

        async fn wait(self: Box<Self>) -> io::Result<ExitStatus> {
            Err(io::Error::other("injected flow wait failure"))
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn successful_status() -> ExitStatus {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }

    fn failed_status() -> ExitStatus {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        #[cfg(unix)]
        let raw = 1 << 8;
        #[cfg(windows)]
        let raw = 1;
        ExitStatus::from_raw(raw)
    }

    fn seeded_execution(
        root: &Path,
        cwd: &Path,
        controller: Arc<dyn ProcessController>,
        runtime: Option<RuntimePolicy>,
    ) -> (Arc<Store>, Scheduler, crate::store::FlowTaskExecution) {
        let paths = StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        };
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        store.lock_queue().unwrap();
        store.set_mode(ExecutionMode::Scheduled).unwrap();
        if let Some(runtime) = runtime {
            store.set_runtime_policy(runtime).unwrap();
        }
        store.unlock_queue().unwrap();
        let flow = store
            .create_flow(
                "flow-execution-test".into(),
                "Flow execution test".into(),
                "test".into(),
                ScheduleSpec::Once {
                    at: Utc::now() + ChronoDuration::hours(1),
                },
            )
            .unwrap();
        store
            .add_flow_task(FlowTaskInput {
                flow_id: flow.flow_id.clone(),
                task_id: "root".into(),
                name: "Root".into(),
                cwd: cwd.to_string_lossy().into_owned(),
                command: "ignored by controller".into(),
                retry: 0,
                dependencies: vec![],
                depend_mode: Default::default(),
            })
            .unwrap();
        store.commit_flow(&flow.flow_id).unwrap();
        let run = store
            .create_flow_run(&flow.flow_id, "MANUAL", false, None)
            .unwrap();
        let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
        assert_eq!(execution.run_id, run.run_id);
        let scheduler = Scheduler::with_controller(paths, Arc::clone(&store), controller);
        (store, scheduler, execution)
    }

    #[tokio::test]
    async fn startup_and_spawn_failures_are_saved_as_distinct_attempt_errors() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing-cwd");
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            &missing,
            Arc::new(TimeoutController),
            None,
        );
        scheduler
            .execute_flow_task(execution.clone())
            .await
            .unwrap();
        let attempt = store
            .list_flow_attempts(execution.run_id, &execution.task_id)
            .unwrap()
            .remove(0);
        assert_eq!(attempt.failure_kind.as_deref(), Some("STARTUP"));

        let directory = tempfile::tempdir().unwrap();
        let file_cwd = directory.path().join("not-a-directory");
        std::fs::write(&file_cwd, b"file").unwrap();
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            &file_cwd,
            Arc::new(TimeoutController),
            None,
        );
        scheduler
            .execute_flow_task(execution.clone())
            .await
            .unwrap();
        let attempt = store
            .list_flow_attempts(execution.run_id, &execution.task_id)
            .unwrap()
            .remove(0);
        assert_eq!(attempt.failure_kind.as_deref(), Some("STARTUP"));

        let directory = tempfile::tempdir().unwrap();
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            directory.path(),
            Arc::new(SpawnFailure),
            None,
        );
        scheduler
            .execute_flow_task(execution.clone())
            .await
            .unwrap();
        let attempt = store
            .list_flow_attempts(execution.run_id, &execution.task_id)
            .unwrap()
            .remove(0);
        assert_eq!(attempt.failure_kind.as_deref(), Some("SPAWN"));
        assert!(
            attempt
                .failure_detail
                .as_deref()
                .unwrap()
                .contains("injected flow spawn failure")
        );
    }

    #[tokio::test]
    async fn process_exit_and_wait_errors_are_persisted_as_failure_or_lost() {
        let directory = tempfile::tempdir().unwrap();
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            directory.path(),
            Arc::new(ExitController),
            None,
        );
        scheduler
            .execute_flow_task(execution.clone())
            .await
            .unwrap();
        let attempt = store
            .list_flow_attempts(execution.run_id, &execution.task_id)
            .unwrap()
            .remove(0);
        assert_eq!(attempt.failure_kind.as_deref(), Some("EXIT"));
        assert_eq!(attempt.exit_code, Some(1));

        let directory = tempfile::tempdir().unwrap();
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            directory.path(),
            Arc::new(WaitFailureController),
            None,
        );
        scheduler
            .execute_flow_task(execution.clone())
            .await
            .unwrap();
        let attempt = store
            .list_flow_attempts(execution.run_id, &execution.task_id)
            .unwrap()
            .remove(0);
        assert_eq!(attempt.state, crate::domain::flow::AttemptState::Lost);
        assert!(
            attempt
                .failure_detail
                .as_deref()
                .unwrap()
                .contains("injected flow wait failure")
        );
    }

    #[tokio::test]
    async fn runtime_timeout_cancels_process_and_records_timeout_failure() {
        let directory = tempfile::tempdir().unwrap();
        let policy = RuntimePolicy {
            max_runtime_ms: Some(1),
            ..RuntimePolicy::default()
        };
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            directory.path(),
            Arc::new(TimeoutController),
            Some(policy),
        );

        scheduler
            .execute_flow_task(execution.clone())
            .await
            .unwrap();
        let attempt = store
            .list_flow_attempts(execution.run_id, &execution.task_id)
            .unwrap()
            .remove(0);
        assert_eq!(attempt.failure_kind.as_deref(), Some("RUNTIME_TIMEOUT"));
        assert_eq!(
            attempt.failure_detail.as_deref(),
            Some("flow task exceeded max runtime")
        );
    }

    #[tokio::test]
    async fn cancellation_request_is_forwarded_to_the_running_process() {
        let directory = tempfile::tempdir().unwrap();
        let (store, scheduler, execution) = seeded_execution(
            directory.path(),
            directory.path(),
            Arc::new(TimeoutController),
            None,
        );
        let flow_id = execution.flow_id.clone();
        let run_id = execution.run_id;
        let attempt_id = execution.attempt_id;
        let scheduler_task = tokio::spawn(async move {
            scheduler.execute_flow_task(execution).await.unwrap();
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let attempt = store.list_flow_attempts(run_id, "root").unwrap().remove(0);
                if attempt.state == crate::domain::flow::AttemptState::Running {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the attempt should reach RUNNING before cancellation");

        store.cancel_flow_task(&flow_id, "root", run_id).unwrap();
        tokio::time::timeout(Duration::from_secs(2), scheduler_task)
            .await
            .expect("the process should stop after receiving the cancellation signal")
            .unwrap();
        let attempt = store.list_flow_attempts(run_id, "root").unwrap().remove(0);
        assert_eq!(attempt.attempt_id, attempt_id);
        assert_eq!(attempt.state, crate::domain::flow::AttemptState::Cancelled);
        assert_eq!(attempt.failure_detail.as_deref(), Some("cancel requested"));
    }
}
