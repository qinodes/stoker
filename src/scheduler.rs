//! FIFO, single-slot execution of committed jobs.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use tokio::sync::{broadcast, watch};
use tokio::time::sleep;
use uuid::Uuid;

use crate::domain::{Job, JobState};
use crate::ipc::{LogStream, ServiceStatus};
use crate::process::{DefaultProcessController, ProcessController, ProcessSpec};
use crate::{StokerPaths, Store, StoreError};

#[derive(Debug, Clone)]
pub(crate) struct LogEvent {
    pub stream: LogStream,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct Scheduler {
    paths: StokerPaths,
    store: Arc<Store>,
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

#[derive(Debug, Clone)]
pub(crate) enum LogMessage {
    Chunk(LogEvent),
    End,
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
        Self {
            paths,
            store,
            controller,
            active: Arc::new(Mutex::new(None)),
            stopping: Arc::new(AtomicBool::new(false)),
            logs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn handle_commit(&self, id: Uuid, wake: &watch::Sender<u64>) -> anyhow::Result<Job> {
        let job = self.store.commit_job(id).context("commit job")?;
        wake.send_modify(|value| *value = value.wrapping_add(1));
        Ok(job)
    }

    pub fn handle_commit_many(
        &self,
        ids: &[Uuid],
        wake: &watch::Sender<u64>,
    ) -> anyhow::Result<Vec<Job>> {
        let jobs = self.store.commit_jobs(ids).context("commit jobs")?;
        if !jobs.is_empty() {
            wake.send_modify(|value| *value = value.wrapping_add(1));
        }
        Ok(jobs)
    }

    pub fn handle_commit_all(&self, wake: &watch::Sender<u64>) -> anyhow::Result<Vec<Job>> {
        let jobs = self.store.commit_all_drafts().context("commit all jobs")?;
        if !jobs.is_empty() {
            wake.send_modify(|value| *value = value.wrapping_add(1));
        }
        Ok(jobs)
    }

    pub fn handle_commit_user(
        &self,
        user: &str,
        wake: &watch::Sender<u64>,
    ) -> anyhow::Result<Vec<Job>> {
        let jobs = self
            .store
            .commit_user_drafts(user)
            .context("commit user jobs")?;
        if !jobs.is_empty() {
            wake.send_modify(|value| *value = value.wrapping_add(1));
        }
        Ok(jobs)
    }

    pub fn handle_lock_queue(&self) -> anyhow::Result<()> {
        self.store.lock_queue().context("lock queue")?;
        Ok(())
    }

    pub fn handle_unlock_queue(&self, wake: &watch::Sender<u64>) -> anyhow::Result<()> {
        self.store.unlock_queue().context("unlock queue")?;
        wake.send_modify(|value| *value = value.wrapping_add(1));
        Ok(())
    }

    pub fn handle_move_queued(&self, id: Uuid, target_order: usize) -> anyhow::Result<Vec<Job>> {
        self.store
            .move_queued_job(id, target_order)
            .map_err(anyhow::Error::from)
    }

    /// Cancel a job, waiting for the active process and runtime cleanup before
    /// acknowledging an active cancellation request.
    pub async fn handle_cancel(&self, id: Uuid) -> anyhow::Result<Job> {
        let job = self
            .store
            .get_job(id)
            .context("inspect job for cancellation")?;
        match job.state {
            JobState::Draft | JobState::Queued => {
                Ok(self.store.cancel_not_started(id).context("cancel job")?)
            }
            JobState::Starting | JobState::Running | JobState::Cancelling => {
                self.cancel_active(id).await
            }
            state => anyhow::bail!("cannot cancel job {id} while it is {state}"),
        }
    }

    /// Request shutdown cancellation for the current execution and wait until
    /// its terminal state and cleanup have been persisted.
    pub async fn stop_active(&self) -> anyhow::Result<()> {
        let id = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler active slot is poisoned"))?
            .as_ref()
            .map(|active| active.id);
        if let Some(id) = id {
            self.cancel_active(id).await?;
        }
        Ok(())
    }

    /// Close the scheduler intake. This is set before the service shutdown
    /// watch is broadcast so a concurrent queue wake cannot claim another job
    /// while Stop is waiting for active cleanup.
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    async fn cancel_active(&self, id: Uuid) -> anyhow::Result<Job> {
        let (active, completed) = {
            let active = self
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("scheduler active slot is poisoned"))?;
            let active = active.as_ref().filter(|active| active.id == id).cloned();
            let completed = active.as_ref().map(|active| active.completed.subscribe());
            (active, completed)
        };
        let Some(active) = active else {
            let current = self.store.get_job(id).context("inspect active job")?;
            if matches!(
                current.state,
                JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
            ) {
                anyhow::bail!("cannot cancel job {id} while it is {}", current.state);
            }
            anyhow::bail!("job {id} is not managed by the active scheduler");
        };
        let mut completed = completed.expect("active completion receiver must exist");
        let current = self.store.get_job(id).context("inspect active job")?;
        if matches!(
            current.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
        ) {
            while !*completed.borrow() {
                completed
                    .changed()
                    .await
                    .with_context(|| format!("wait for completed job {id}"))?;
            }
            return self.store.get_job(id).context("inspect completed job");
        }
        if current.state != JobState::Cancelling
            && let Err(error) = self.store.request_cancelling(id)
        {
            // The process may finish between the state check above and the
            // transition. Treat that terminal race, or an already requested
            // cancellation, as a successful stop.
            let current = self.store.get_job(id).context("inspect active job")?;
            if !matches!(
                current.state,
                JobState::Succeeded
                    | JobState::Failed
                    | JobState::Cancelled
                    | JobState::Lost
                    | JobState::Cancelling
            ) {
                return Err(error).context(format!(
                    "mark job cancelling while job is {}",
                    current.state
                ));
            }
            if current.state == JobState::Cancelling {
                let _ = active.cancel.send(true);
            }
            while !*completed.borrow() {
                completed
                    .changed()
                    .await
                    .with_context(|| format!("wait for completed job {id}"))?;
            }
            return self.store.get_job(id).context("inspect completed job");
        }
        let _ = active.cancel.send(true);
        loop {
            let current = self.store.get_job(id).context("inspect cancelled job")?;
            if *completed.borrow()
                && matches!(
                    current.state,
                    JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
                )
            {
                return Ok(current);
            }
            // `watch` retains the completion value, so a completion racing
            // with this check cannot be lost between polling and awaiting.
            if completed.changed().await.is_err() {
                anyhow::bail!("scheduler completion signal closed before cleanup for job {id}");
            }
        }
    }

    pub fn service_status(&self) -> anyhow::Result<ServiceStatus> {
        let jobs = self.store.list_jobs(None)?;
        let queue_locked = self.store.queue_locked()?;
        let active_job = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler active slot is poisoned"))?
            .as_ref()
            .map(|active| active.id)
            .or_else(|| {
                jobs.iter()
                    .find(|job| {
                        matches!(
                            job.state,
                            JobState::Starting | JobState::Running | JobState::Cancelling
                        )
                    })
                    .map(|job| job.id)
            });
        Ok(ServiceStatus {
            pid: std::process::id(),
            active_job,
            queued_jobs: jobs
                .iter()
                .filter(|job| job.state == JobState::Queued)
                .count(),
            queue_locked,
        })
    }

    pub(crate) fn log_paths(&self, id: Uuid) -> (PathBuf, PathBuf) {
        let run = self.paths.runs.join(id.to_string());
        (run.join("stdout.log"), run.join("stderr.log"))
    }

    pub(crate) fn log_receiver(&self, id: Uuid) -> Option<broadcast::Receiver<LogMessage>> {
        self.logs
            .lock()
            .ok()
            .and_then(|logs| logs.get(&id).map(broadcast::Sender::subscribe))
    }

    pub(crate) fn job_exists(&self, id: Uuid) -> anyhow::Result<Job> {
        Ok(self.store.get_job(id)?)
    }

    pub async fn run(
        self: Arc<Self>,
        mut wake: watch::Receiver<u64>,
        mut shutdown: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        loop {
            if *shutdown.borrow() || self.stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(job) = self.store.claim_next()? {
                let (cancel, cancel_rx) = watch::channel(false);
                let (completed, _) = watch::channel(false);
                if let Ok(mut active) = self.active.lock() {
                    *active = Some(ActiveExecution {
                        id: job.id,
                        cancel,
                        completed: completed.clone(),
                    });
                }
                let result = self.execute(job, cancel_rx).await;
                if let Ok(mut active) = self.active.lock() {
                    *active = None;
                }
                let _ = completed.send(true);
                // An unrecoverable runtime cleanup error keeps the slot
                // occupied and stops queue progression.
                result?;
                continue;
            }
            tokio::select! {
                changed = wake.changed() => {
                    if changed.is_err() { return Ok(()); }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
            }
        }
    }

    async fn execute(&self, job: Job, mut cancel: watch::Receiver<bool>) -> anyhow::Result<()> {
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
            let running = self.store.set_running(job.id, pid);
            if let Err(error) = running {
                let _ = process_cancel_tx.take().map(|sender| sender.send(()));
                let _ = (&mut process_task).await;
                if self
                    .store
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
                        self.store
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
            self.store
                .finish(job.id, code, None)
                .context("record job result")?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let mut diagnostics = Vec::new();
        if let Err(error) = result {
            let detail = format!("{error:#}");
            diagnostics.push(detail.clone());
            if let Err(finish_error) = self.store.finish(job.id, None, Some(&detail)) {
                diagnostics.push(format!("record FAILED result: {finish_error}"));
            }
        }
        let mut cleanup_failed = false;
        let mut cleared = false;
        let mut clear_error = None;
        let mut job_removed = false;
        for _ in 0..3 {
            match self.store.clear_runtime(job.id) {
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
            if let Err(error) = self.store.record_failure_detail(job.id, &detail) {
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

async fn watch_logs(stdout: PathBuf, stderr: PathBuf, sender: broadcast::Sender<LogMessage>) {
    let mut offsets = [0_u64, 0_u64];
    loop {
        let paths = [&stdout, &stderr];
        for (index, path) in paths.iter().enumerate() {
            if let Ok(bytes) = tokio::fs::read(path).await {
                let offset = offsets[index] as usize;
                if bytes.len() > offset {
                    let chunk = bytes[offset..].to_vec();
                    offsets[index] = bytes.len() as u64;
                    let _ = sender.send(LogMessage::Chunk(LogEvent {
                        stream: if index == 0 {
                            LogStream::Stdout
                        } else {
                            LogStream::Stderr
                        },
                        offset: offset as u64,
                        bytes: chunk,
                    }));
                }
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
}

async fn flush_log_events(stdout: &Path, stderr: &Path, sender: &broadcast::Sender<LogMessage>) {
    for (path, stream) in [(stdout, LogStream::Stdout), (stderr, LogStream::Stderr)] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            let _ = sender.send(LogMessage::Chunk(LogEvent {
                stream,
                offset: 0,
                bytes,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ManagedProcess;
    use crate::{NewJob, StokerPaths, Store};
    use async_trait::async_trait;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct FailingWaitController;

    struct FailingWaitProcess;

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
    async fn failed_setup_and_spawn_are_persisted_as_failed_jobs() {
        let (directory, store, scheduler) = scheduler_fixture();
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
        let spawn_failure = store
            .create_job(NewJob {
                name: "spawn failure".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["program-that-does-not-exist".into()],
            })
            .unwrap();
        for id in [missing_cwd, empty_command, spawn_failure] {
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
                if states.len() == 3 && states.iter().all(|state| matches!(state, JobState::Failed))
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

        for id in [missing_cwd, empty_command, spawn_failure] {
            let job = store.get_job(id).unwrap();
            assert_eq!(job.state, JobState::Failed);
            assert!(job.failure_detail.is_some());
            assert_eq!(job.pid, None);
        }
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

        let status = scheduler.service_status().unwrap();
        assert_eq!(status.active_job, None);
        assert_eq!(status.queued_jobs, 1);
        store.lock_queue().unwrap();
        assert!(scheduler.service_status().unwrap().queue_locked);
        store.unlock_queue().unwrap();
        store.claim_next().unwrap();
        assert_eq!(scheduler.service_status().unwrap().active_job, Some(id));
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
        let (directory, store, scheduler) = scheduler_fixture();
        let id = store
            .create_job(NewJob {
                name: "active".into(),
                user: "test".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: if cfg!(windows) {
                    vec![
                        "cmd".into(),
                        "/C".into(),
                        "ping 127.0.0.1 -n 31 > NUL".into(),
                    ]
                } else {
                    vec!["sh".into(), "-c".into(), "sleep 30".into()]
                },
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
        let task = tokio::spawn(watch_logs(stdout, stderr, sender));

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
            LogMessage::Chunk(LogEvent { stream: LogStream::Stdout, offset: 0, bytes })
                if bytes == b"out"
        )));
        assert!(events.iter().any(|message| matches!(
            message,
            LogMessage::Chunk(LogEvent { stream: LogStream::Stderr, offset: 0, bytes })
                if bytes == b"err"
        )));
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
        flush_log_events(&stdout, &stderr, &sender).await;

        let message = receiver.recv().await.unwrap();
        assert!(matches!(
            message,
            LogMessage::Chunk(LogEvent { stream: LogStream::Stdout, offset: 0, bytes })
                if bytes == b"already there"
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
