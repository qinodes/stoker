//! FIFO, single-slot execution of committed jobs.

mod cancellation;
mod control;
mod execution;
mod flow_execution;
mod logs;
pub mod model;
mod runner;
mod status;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::process::{DefaultProcessController, ProcessController};
use crate::{StokerPaths, Store};

use execution::ExecutionStore;

pub(crate) use model::{LogEvent, LogMessage};
pub use model::{OutputStream, SchedulerStatus};

/// Maximum number of bytes carried by one durable/live log event.
pub(crate) const LOG_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone)]
pub struct Scheduler {
    paths: StokerPaths,
    store: Arc<Store>,
    execution_store: Arc<dyn ExecutionStore>,
    controller: Arc<dyn ProcessController>,
    active: Arc<Mutex<Option<ActiveExecution>>>,
    stopping: Arc<AtomicBool>,
    logs: Arc<Mutex<HashMap<Uuid, broadcast::Sender<LogMessage>>>>,
}

#[derive(Clone)]
struct ActiveExecution {
    id: Uuid,
    cancel: watch::Sender<bool>,
    completed: watch::Sender<bool>,
}

impl Scheduler {
    pub fn new(paths: StokerPaths, store: Arc<Store>) -> Self {
        Self::with_controller(paths, store, Arc::new(DefaultProcessController::new()))
    }

    pub(crate) fn with_controller(
        paths: StokerPaths,
        store: Arc<Store>,
        controller: Arc<dyn ProcessController>,
    ) -> Self {
        let execution_store: Arc<dyn ExecutionStore> = store.clone();
        Self::with_dependencies(paths, store, controller, execution_store)
    }

    fn with_dependencies(
        paths: StokerPaths,
        store: Arc<Store>,
        controller: Arc<dyn ProcessController>,
        execution_store: Arc<dyn ExecutionStore>,
    ) -> Self {
        Self {
            paths,
            store,
            execution_store,
            controller,
            active: Arc::new(Mutex::new(None)),
            stopping: Arc::new(AtomicBool::new(false)),
            logs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::logs::{flush_log_events, watch_logs};
    use super::*;
    use crate::process::{ManagedProcess, ProcessSpec};
    use crate::{JobState, NewJob, StokerPaths, Store};
    use async_trait::async_trait;
    use std::io;
    use std::path::PathBuf;
    use std::process::ExitStatus;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    struct FailingWaitController;

    struct FailingWaitProcess;

    struct SuccessfulController;
    struct FailingSpawnController;
    struct ImmediateProcess;
    struct CancelAwareController;
    struct CancelAwareProcess;

    struct FailingExecutionStore {
        store: Arc<Store>,
        finish_failures: AtomicUsize,
        cleanup_failures: AtomicUsize,
    }

    impl FailingExecutionStore {
        fn new(store: Arc<Store>, finish_failures: usize, cleanup_failures: usize) -> Self {
            Self {
                store,
                finish_failures: AtomicUsize::new(finish_failures),
                cleanup_failures: AtomicUsize::new(cleanup_failures),
            }
        }
    }

    impl ExecutionStore for FailingExecutionStore {
        fn get_job(&self, id: uuid::Uuid) -> Result<crate::Job, crate::StoreError> {
            self.store.get_job(id)
        }

        fn set_running(&self, id: uuid::Uuid, pid: u32) -> Result<crate::Job, crate::StoreError> {
            self.store.set_running(id, pid)
        }

        fn finish(
            &self,
            id: uuid::Uuid,
            exit_code: Option<i32>,
            failure_detail: Option<&str>,
        ) -> Result<crate::Job, crate::StoreError> {
            if self
                .finish_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(crate::StoreError::InvalidData(
                    "injected finish failure".to_owned(),
                ));
            }
            self.store.finish(id, exit_code, failure_detail)
        }

        fn clear_runtime(&self, id: uuid::Uuid) -> Result<crate::Job, crate::StoreError> {
            if self
                .cleanup_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(crate::StoreError::InvalidData(
                    "injected cleanup failure".to_owned(),
                ));
            }
            self.store.clear_runtime(id)
        }

        fn record_failure_detail(
            &self,
            id: uuid::Uuid,
            detail: &str,
        ) -> Result<crate::Job, crate::StoreError> {
            self.store.record_failure_detail(id, detail)
        }
    }

    fn successful_status() -> ExitStatus {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }

    #[async_trait]
    impl ProcessController for FailingWaitController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(FailingWaitProcess))
        }
    }

    #[async_trait]
    impl ManagedProcess for FailingWaitProcess {
        fn pid(&self) -> u32 {
            4242
        }

        async fn wait(self: Box<Self>) -> io::Result<std::process::ExitStatus> {
            Err(io::Error::other("mock process wait failed"))
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ProcessController for SuccessfulController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(ImmediateProcess))
        }
    }

    #[async_trait]
    impl ProcessController for FailingSpawnController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Err(io::Error::other("injected spawn failure"))
        }
    }

    #[async_trait]
    impl ProcessController for CancelAwareController {
        async fn spawn(&self, _spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(CancelAwareProcess))
        }
    }

    #[async_trait]
    impl ManagedProcess for ImmediateProcess {
        fn pid(&self) -> u32 {
            4242
        }

        async fn wait(self: Box<Self>) -> io::Result<ExitStatus> {
            Ok(successful_status())
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ManagedProcess for CancelAwareProcess {
        fn pid(&self) -> u32 {
            4242
        }

        async fn wait(self: Box<Self>) -> io::Result<ExitStatus> {
            Ok(successful_status())
        }

        async fn wait_with_cancel(
            self: Box<Self>,
            cancel: tokio::sync::oneshot::Receiver<()>,
        ) -> io::Result<ExitStatus> {
            let _ = cancel.await;
            Ok(successful_status())
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn paths(root: &std::path::Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    fn scheduler_fixture() -> (tempfile::TempDir, Arc<Store>, Scheduler) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let scheduler = Scheduler::new(paths, Arc::clone(&store));
        (directory, store, scheduler)
    }

    fn scheduler_fixture_with_controller(
        controller: Arc<dyn ProcessController>,
    ) -> (tempfile::TempDir, Arc<Store>, Scheduler) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let scheduler = Scheduler::with_controller(paths, Arc::clone(&store), controller);
        (directory, store, scheduler)
    }

    #[tokio::test]
    async fn completion_watch_retains_signal_if_sent_before_wait() {
        let (completed, mut receiver) = watch::channel(false);
        completed.send(true).unwrap();

        // This models execution completing between a durable-state check and
        // registration of the waiter. Unlike Notify, watch retains the value
        // and cannot strand cancellation or stop waiting forever.
        assert!(receiver.changed().await.is_ok());
        assert!(*receiver.borrow());
    }

    #[tokio::test]
    async fn scheduler_run_exits_immediately_when_shutdown_is_set() {
        let (_directory, store, scheduler) = scheduler_fixture();
        let (_wake_tx, wake_rx) = watch::channel(0_u64);
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        Arc::new(scheduler).run(wake_rx, shutdown_rx).await.unwrap();
        assert!(store.list_jobs(None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_setup_is_persisted_without_starting_a_real_process() {
        let (directory, store, scheduler) =
            scheduler_fixture_with_controller(Arc::new(SuccessfulController));
        let missing_cwd = store
            .create_job(NewJob {
                name: "missing cwd".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().join("does-not-exist"),
                command: vec!["echo".into()],
            })
            .unwrap();
        let empty_command = store
            .create_job(NewJob {
                name: "empty command".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: Vec::new(),
            })
            .unwrap();
        for id in [missing_cwd, empty_command] {
            store.commit_job(id).unwrap();
        }

        let scheduler = Arc::new(scheduler);
        let (wake_tx, wake_rx) = watch::channel(0_u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let states = store
                    .list_jobs(None)
                    .unwrap()
                    .into_iter()
                    .map(|job| job.state)
                    .collect::<Vec<_>>();
                if states.len() == 2 && states.iter().all(|state| matches!(state, JobState::Failed))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("scheduler should finish failed jobs");
        shutdown_tx.send(true).unwrap();
        drop(wake_tx);
        scheduler_task.await.unwrap().unwrap();

        for id in [missing_cwd, empty_command] {
            let job = store.get_job(id).unwrap();
            assert_eq!(job.state, JobState::Failed);
            assert!(job.failure_detail.is_some());
            assert_eq!(job.pid, None);
        }
    }

    #[tokio::test]
    async fn fake_spawn_failure_is_persisted_as_a_failed_job() {
        let (directory, store, scheduler) =
            scheduler_fixture_with_controller(Arc::new(FailingSpawnController));
        let id = store
            .create_job(NewJob {
                name: "spawn failure".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["ignored-by-fake".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();

        let scheduler = Arc::new(scheduler);
        let (wake_tx, wake_rx) = watch::channel(0_u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));
        wake_tx.send_modify(|value| *value += 1);

        tokio::time::timeout(Duration::from_secs(2), async {
            while {
                let job = store.get_job(id).unwrap();
                job.state != JobState::Failed || job.pid.is_some()
            } {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fake spawn failure should finish the job");
        let job = store.get_job(id).unwrap();
        assert_eq!(job.pid, None);
        assert!(
            job.failure_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("injected spawn failure"))
        );

        shutdown_tx.send(true).unwrap();
        scheduler_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn mocked_process_wait_failure_is_persisted_as_a_failed_job() {
        let (directory, store, scheduler) =
            scheduler_fixture_with_controller(Arc::new(FailingWaitController));
        let id = store
            .create_job(NewJob {
                name: "wait failure".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "mocked".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();

        let scheduler = Arc::new(scheduler);
        let (wake_tx, wake_rx) = watch::channel(0_u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));
        wake_tx.send_modify(|value| *value += 1);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if store.get_job(id).unwrap().state == JobState::Failed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("mock process wait failure should finish the job");

        let job = store.get_job(id).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.pid, None);
        assert!(
            job.failure_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("mock process wait failed"))
        );

        shutdown_tx.send(true).unwrap();
        drop(wake_tx);
        scheduler_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn finalize_failure_is_retried_as_a_durable_failed_result() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let execution_store: Arc<dyn ExecutionStore> =
            Arc::new(FailingExecutionStore::new(Arc::clone(&store), 1, 0));
        let scheduler = Arc::new(Scheduler::with_dependencies(
            paths,
            Arc::clone(&store),
            Arc::new(SuccessfulController),
            execution_store,
        ));
        let id = store
            .create_job(NewJob {
                name: "finalize failure".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["ignored-by-fake".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();
        let (_wake_tx, wake_rx) = watch::channel(0_u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));

        tokio::time::timeout(Duration::from_secs(2), async {
            while {
                let job = store.get_job(id).unwrap();
                job.state != JobState::Failed || job.pid.is_some()
            } {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fallback finalization should persist FAILED");
        let job = store.get_job(id).unwrap();
        assert_eq!(job.pid, None);
        assert!(
            job.failure_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("injected finish failure"))
        );

        shutdown_tx.send(true).unwrap();
        scheduler_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cleanup_failure_stops_queue_progression_and_keeps_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let execution_store: Arc<dyn ExecutionStore> =
            Arc::new(FailingExecutionStore::new(Arc::clone(&store), 0, 3));
        let scheduler = Arc::new(Scheduler::with_dependencies(
            paths,
            Arc::clone(&store),
            Arc::new(SuccessfulController),
            execution_store,
        ));
        let id = store
            .create_job(NewJob {
                name: "cleanup failure".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["ignored-by-fake".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();
        let (_wake_tx, wake_rx) = watch::channel(0_u64);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);

        let error =
            tokio::time::timeout(Duration::from_secs(2), scheduler.run(wake_rx, shutdown_rx))
                .await
                .expect("cleanup retries must be bounded")
                .unwrap_err();
        assert!(error.to_string().contains("cleanup incomplete"));
        let job = store.get_job(id).unwrap();
        assert_eq!(job.state, JobState::Succeeded);
        assert_eq!(job.pid, Some(4242));
        assert!(
            job.failure_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("injected cleanup failure"))
        );
    }

    #[tokio::test]
    async fn scheduler_status_tracks_queue_lock_and_active_fallback() {
        let (_directory, store, scheduler) = scheduler_fixture();
        let id = store
            .create_job(NewJob {
                name: "queued".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();

        let status = scheduler.scheduler_status().unwrap();
        assert_eq!(status.active_job, None);
        assert_eq!(status.queued_jobs, 1);
        store.lock_queue().unwrap();
        assert!(scheduler.scheduler_status().unwrap().queue_locked);
        store.unlock_queue().unwrap();
        store.claim_next().unwrap();
        assert_eq!(scheduler.scheduler_status().unwrap().active_job, Some(id));
    }

    #[tokio::test]
    async fn cancellation_handles_not_started_and_terminal_jobs() {
        let (_directory, store, scheduler) = scheduler_fixture();
        let draft = store
            .create_job(NewJob {
                name: "draft".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        assert_eq!(
            scheduler.handle_cancel(draft).await.unwrap().state,
            JobState::Cancelled
        );

        let queued = store
            .create_job(NewJob {
                name: "queued".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.commit_job(queued).unwrap();
        assert_eq!(
            scheduler.handle_cancel(queued).await.unwrap().state,
            JobState::Cancelled
        );

        let error = scheduler.handle_cancel(draft).await.unwrap_err();
        assert!(error.to_string().contains("cannot cancel job"));
    }

    #[tokio::test]
    async fn active_cancellation_waits_for_cleanup_and_clears_runtime_fields() {
        let (directory, store, scheduler) =
            scheduler_fixture_with_controller(Arc::new(CancelAwareController));
        let id = store
            .create_job(NewJob {
                name: "active".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["ignored-by-fake".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();

        let scheduler = Arc::new(scheduler);
        let (wake_tx, wake_rx) = watch::channel(0_u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));
        wake_tx.send_modify(|value| *value += 1);

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if store.get_job(id).unwrap().state == JobState::Running {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("job should reach RUNNING");

        // Exercise the already-requested cancellation path. The scheduler
        // still owns the active process and must perform the actual cleanup.
        store.request_cancelling(id).unwrap();
        let cancelled = scheduler.handle_cancel(id).await.unwrap();
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(cancelled.pid, None);

        shutdown_tx.send(true).unwrap();
        scheduler_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancellation_of_an_active_slot_with_terminal_state_returns_persisted_job() {
        let (_directory, store, scheduler) = scheduler_fixture();
        let id = store
            .create_job(NewJob {
                name: "cancelled".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        let expected = store.cancel_not_started(id).unwrap();
        let (cancel, _) = watch::channel(false);
        let (completed, _) = watch::channel(true);
        *scheduler.active.lock().unwrap() = Some(ActiveExecution {
            id,
            cancel,
            completed,
        });

        let actual = scheduler.cancel_active(id).await.unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn log_watchers_publish_stdout_and_stderr_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let stdout = directory.path().join("stdout.log");
        let stderr = directory.path().join("stderr.log");
        tokio::fs::write(&stdout, b"out").await.unwrap();
        tokio::fs::write(&stderr, b"err").await.unwrap();
        let (sender, mut receiver) = broadcast::channel(8);
        let offsets = Arc::new(tokio::sync::Mutex::new([0_u64, 0_u64]));
        let task = tokio::spawn(watch_logs(stdout, stderr, sender, offsets));

        let first = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let second = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let events = [first, second];
        assert!(events.iter().any(|message| matches!(
            message,
            LogMessage::Chunk(LogEvent { stream: OutputStream::Stdout, offset: 0, bytes })
                if bytes == b"out"
        )));
        assert!(events.iter().any(|message| matches!(
            message,
            LogMessage::Chunk(LogEvent { stream: OutputStream::Stderr, offset: 0, bytes })
                if bytes == b"err"
        )));
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn log_watcher_splits_large_files_into_bounded_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let stdout = directory.path().join("stdout.log");
        let stderr = directory.path().join("stderr.log");
        let payload = vec![b'x'; LOG_CHUNK_SIZE * 2 + 17];
        tokio::fs::write(&stdout, &payload).await.unwrap();
        tokio::fs::write(&stderr, b"").await.unwrap();
        let (sender, mut receiver) = broadcast::channel(8);
        let offsets = Arc::new(tokio::sync::Mutex::new([0_u64, 0_u64]));
        let task = tokio::spawn(watch_logs(stdout, stderr, sender, offsets));

        let mut chunks = Vec::new();
        while chunks.len() < 3 {
            let message = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            if let LogMessage::Chunk(event) = message {
                assert_eq!(event.stream, OutputStream::Stdout);
                assert!(event.bytes.len() <= LOG_CHUNK_SIZE);
                chunks.push(event);
            }
        }
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[1].offset, LOG_CHUNK_SIZE as u64);
        assert_eq!(chunks[2].offset, (LOG_CHUNK_SIZE * 2) as u64);
        assert_eq!(
            chunks.iter().map(|chunk| chunk.bytes.len()).sum::<usize>(),
            payload.len()
        );
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn flush_log_events_sends_existing_files_and_skips_missing_files() {
        let directory = tempfile::tempdir().unwrap();
        let stdout = directory.path().join("stdout.log");
        let stderr = directory.path().join("stderr.log");
        tokio::fs::write(&stdout, b"already there").await.unwrap();
        let (sender, mut receiver) = broadcast::channel(4);
        let offsets = Arc::new(tokio::sync::Mutex::new([0_u64, 0_u64]));
        flush_log_events(&stdout, &stderr, &sender, &offsets).await;

        let message = receiver.recv().await.unwrap();
        assert!(matches!(
            message,
            LogMessage::Chunk(LogEvent { stream: OutputStream::Stdout, offset: 0, bytes })
                if bytes == b"already there"
        ));
    }

    #[tokio::test]
    async fn flush_log_events_splits_large_files_into_bounded_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let stdout = directory.path().join("stdout.log");
        let stderr = directory.path().join("stderr.log");
        let payload = vec![b'y'; LOG_CHUNK_SIZE + 9];
        tokio::fs::write(&stdout, &payload).await.unwrap();
        let (sender, mut receiver) = broadcast::channel(8);
        let offsets = Arc::new(tokio::sync::Mutex::new([0_u64, 0_u64]));
        flush_log_events(&stdout, &stderr, &sender, &offsets).await;

        let first = receiver.recv().await.unwrap();
        let second = receiver.recv().await.unwrap();
        assert!(matches!(
            first,
            LogMessage::Chunk(LogEvent { offset: 0, bytes, .. })
                if bytes.len() == LOG_CHUNK_SIZE
        ));
        assert!(matches!(
            second,
            LogMessage::Chunk(LogEvent { offset, bytes, .. })
                if offset == LOG_CHUNK_SIZE as u64 && bytes.len() == 9
        ));
    }

    #[tokio::test]
    async fn flush_log_events_only_publishes_bytes_after_watcher_offset() {
        let directory = tempfile::tempdir().unwrap();
        let stdout = directory.path().join("stdout.log");
        let stderr = directory.path().join("stderr.log");
        tokio::fs::write(&stdout, b"already there").await.unwrap();
        tokio::fs::write(&stderr, b"").await.unwrap();
        let offsets = Arc::new(tokio::sync::Mutex::new([13_u64, 0_u64]));
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&stdout)
            .await
            .unwrap()
            .write_all(b"tail")
            .await
            .unwrap();
        let (sender, mut receiver) = broadcast::channel(4);
        flush_log_events(&stdout, &stderr, &sender, &offsets).await;

        let message = receiver.recv().await.unwrap();
        assert!(matches!(
            message,
            LogMessage::Chunk(LogEvent { stream: OutputStream::Stdout, offset: 13, bytes })
                if bytes == b"tail"
        ));
    }

    #[test]
    fn scheduler_helpers_expose_log_paths_and_missing_log_receivers() {
        let (_directory, _store, scheduler) = scheduler_fixture();
        let id = Uuid::nil();
        let (stdout, stderr) = scheduler.log_paths(id);
        let expected_run = PathBuf::from(id.to_string());
        assert!(stdout.ends_with(expected_run.join("stdout.log")));
        assert!(stderr.ends_with(expected_run.join("stderr.log")));
        assert!(scheduler.log_receiver(id).is_none());
        assert!(scheduler.job_exists(id).is_err());
    }
}
