//! Singleton service bootstrap and IPC request handling.

use std::fs::{File, OpenOptions};
use std::sync::Arc;

use anyhow::Context;
use fs2::FileExt;
use tokio::sync::{mpsc, watch};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::domain::JobState;
use crate::ipc::{
    IpcRequest, IpcResponse, LogStream, ServiceStatus, decode_request, send_response,
};
use crate::scheduler::{LogMessage, Scheduler};
use crate::{StokerPaths, Store, StoreError};

pub struct Service {
    paths: StokerPaths,
    store: Arc<Store>,
    // Keeping this value in the Service struct retains the OS lock for the
    // complete lifetime of the listener. It is released only after run exits.
    _lock: File,
}

impl Service {
    pub fn new(paths: StokerPaths) -> anyhow::Result<Self> {
        paths.ensure()?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&paths.lock)
            .with_context(|| format!("open service lock {}", paths.lock.display()))?;
        lock.try_lock_exclusive().map_err(|error| {
            anyhow::anyhow!(
                "scheduler service is already running (could not acquire {}) : {error}",
                paths.lock.display()
            )
        })?;
        let store = Arc::new(Store::open(&paths.database).context("open scheduler database")?);
        Ok(Self {
            paths,
            store,
            _lock: lock,
        })
    }

    pub fn status(&self) -> anyhow::Result<ServiceStatus> {
        let jobs = self.store.list_jobs(None)?;
        let queue_locked = self.store.queue_locked()?;
        let active_job = jobs
            .iter()
            .find(|job| {
                matches!(
                    job.state,
                    JobState::Starting | JobState::Running | JobState::Cancelling
                )
            })
            .map(|job| job.id);
        let queued_jobs = jobs
            .iter()
            .filter(|job| job.state == JobState::Queued)
            .count();
        Ok(ServiceStatus {
            pid: std::process::id(),
            active_job,
            queued_jobs,
            queue_locked,
        })
    }

    /// Run the service until a Stop request is received.
    pub async fn run(self) -> anyhow::Result<()> {
        // A service never reattaches to processes from a previous instance.
        // Recover before the scheduler can claim any new queue work.
        self.store
            .mark_runtime_jobs_lost()
            .context("recover interrupted jobs")?;
        let scheduler = std::sync::Arc::new(Scheduler::new(
            self.paths.clone(),
            std::sync::Arc::clone(&self.store),
        ));
        let (wake_tx, wake_rx) = watch::channel(0_u64);
        let scheduler_wake = wake_rx.clone();
        #[cfg(unix)]
        return self.run_unix(scheduler, wake_tx, scheduler_wake).await;
        #[cfg(windows)]
        return self.run_windows(scheduler, wake_tx, scheduler_wake).await;
    }

    #[cfg(unix)]
    async fn run_unix(
        self,
        scheduler: std::sync::Arc<Scheduler>,
        wake_tx: watch::Sender<u64>,
        wake_rx: watch::Receiver<u64>,
    ) -> anyhow::Result<()> {
        let endpoint = &self.paths.endpoint;
        remove_stale_socket(endpoint)?;
        let listener = tokio::net::UnixListener::bind(endpoint)
            .with_context(|| format!("bind IPC endpoint {}", endpoint.display()))?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (scheduler_error_tx, scheduler_error_rx) = mpsc::unbounded_channel();
        let scheduler_for_task = scheduler.clone();
        let scheduler_shutdown_rx = shutdown_rx.clone();
        let scheduler_task = tokio::spawn(async move {
            let result = scheduler_for_task.run(wake_rx, scheduler_shutdown_rx).await;
            if let Err(error) = &result {
                let _ = scheduler_error_tx.send(error.to_string());
            }
            result
        });
        let result = self
            .accept_unix(
                listener,
                shutdown_tx,
                shutdown_rx,
                scheduler,
                wake_tx,
                scheduler_error_rx,
            )
            .await;
        let scheduler_result = scheduler_task
            .await
            .map_err(|error| anyhow::anyhow!("scheduler task failed: {error}"))?;
        std::fs::remove_file(endpoint)
            .with_context(|| format!("remove IPC endpoint {}", endpoint.display()))?;
        result.and(scheduler_result)
    }

    #[cfg(unix)]
    async fn accept_unix(
        &self,
        listener: tokio::net::UnixListener,
        shutdown_tx: watch::Sender<bool>,
        mut shutdown_rx: watch::Receiver<bool>,
        scheduler: std::sync::Arc<Scheduler>,
        wake_tx: watch::Sender<u64>,
        mut scheduler_errors: mpsc::UnboundedReceiver<String>,
    ) -> anyhow::Result<()> {
        let mut handlers = Vec::new();
        let result = loop {
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() {
                        break Ok(());
                    }
                },
                scheduler_error = scheduler_errors.recv() => {
                    let error = match scheduler_error {
                        Some(error) => error,
                        None if *shutdown_rx.borrow() => break Ok(()),
                        None => "scheduler task ended without a result (possible panic)".to_owned(),
                    };
                    let _ = shutdown_tx.send(true);
                    break Err(anyhow::anyhow!("scheduler task failed: {error}"));
                },
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            handlers.push(tokio::spawn(handle_client(
                                stream,
                                Arc::clone(&scheduler),
                                wake_tx.clone(),
                                shutdown_tx.clone(),
                                shutdown_rx.clone(),
                            )));
                        }
                        Err(error) => break Err(anyhow::Error::new(error).context("accept scheduler IPC client")),
                    }
                }
            }
        };
        let _ = shutdown_tx.send(true);
        for handler in handlers {
            let _ = handler.await;
        }
        result
    }

    #[cfg(windows)]
    async fn run_windows(
        self,
        scheduler: std::sync::Arc<Scheduler>,
        wake_tx: watch::Sender<u64>,
        wake_rx: watch::Receiver<u64>,
    ) -> anyhow::Result<()> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = self.paths.ipc_endpoint();
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let (scheduler_error_tx, mut scheduler_error_rx) = mpsc::unbounded_channel();
        let scheduler_for_task = scheduler.clone();
        let scheduler_shutdown_rx = shutdown_rx.clone();
        let scheduler_task = tokio::spawn(async move {
            let result = scheduler_for_task.run(wake_rx, scheduler_shutdown_rx).await;
            if let Err(error) = &result {
                let _ = scheduler_error_tx.send(error.to_string());
            }
            result
        });
        let mut handlers = Vec::new();
        let result = loop {
            let server = match ServerOptions::new().create(&name) {
                Ok(server) => server,
                Err(error) => {
                    break Err(
                        anyhow::Error::new(error).context(format!("create IPC endpoint {name}"))
                    );
                }
            };
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() {
                        break Ok(());
                    }
                },
                scheduler_error = scheduler_error_rx.recv() => {
                    let error = match scheduler_error {
                        Some(error) => error,
                        None if *shutdown_rx.borrow() => break Ok(()),
                        None => "scheduler task ended without a result (possible panic)".to_owned(),
                    };
                    let _ = shutdown_tx.send(true);
                    break Err(anyhow::anyhow!("scheduler task failed: {error}"));
                },
                connected = server.connect() => {
                    match connected {
                        Ok(()) => {
                            handlers.push(tokio::spawn(handle_client(
                                server,
                                Arc::clone(&scheduler),
                                wake_tx.clone(),
                                shutdown_tx.clone(),
                                shutdown_rx.clone(),
                            )));
                        }
                        Err(error) => break Err(anyhow::Error::new(error).context("accept scheduler IPC client")),
                    }
                }
            }
        };
        let _ = shutdown_tx.send(true);
        for handler in handlers {
            let _ = handler.await;
        }
        let scheduler_result = scheduler_task
            .await
            .map_err(|error| anyhow::anyhow!("scheduler task failed: {error}"))?;
        result.and(scheduler_result)
    }
}

#[cfg(unix)]
fn remove_stale_socket(endpoint: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    // Coordinate endpoint replacement with any other Stoker process touching
    // this directory. The service singleton lock handles normal startup, but
    // this narrower lock keeps inspect+unlink one critical section for stale
    // endpoint cleanup and makes path replacement by another Stoker process
    // impossible during the operation.
    let parent = endpoint
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let directory_lock = OpenOptions::new()
        .read(true)
        .open(parent)
        .with_context(|| format!("open IPC directory {}", parent.display()))?;
    directory_lock
        .lock_exclusive()
        .with_context(|| format!("lock IPC directory {}", parent.display()))?;

    let metadata = match std::fs::symlink_metadata(endpoint) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect IPC endpoint {}", endpoint.display()));
        }
    };
    if !metadata.file_type().is_socket() {
        anyhow::bail!(
            "refusing to replace non-socket IPC endpoint {}",
            endpoint.display()
        );
    }

    // Re-lstat and compare the inode immediately before unlinking. If another
    // process swaps the path between the first check and this operation, do
    // not remove the replacement; the service will fail its bind and report a
    // clear startup error instead.
    let current = std::fs::symlink_metadata(endpoint)
        .with_context(|| format!("recheck IPC endpoint {}", endpoint.display()))?;
    if !current.file_type().is_socket()
        || current.dev() != metadata.dev()
        || current.ino() != metadata.ino()
    {
        anyhow::bail!("IPC endpoint changed while checking {}", endpoint.display());
    }
    std::fs::remove_file(endpoint)
        .with_context(|| format!("remove stale IPC endpoint {}", endpoint.display()))?;
    Ok(())
}

async fn handle_client<S>(
    stream: S,
    scheduler: Arc<Scheduler>,
    wake_tx: watch::Sender<u64>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    loop {
        let frame = tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return;
                }
                continue;
            }
            frame = futures_util::StreamExt::next(&mut framed) => frame,
        };
        let Some(frame) = frame else { return };
        let frame = match frame {
            Ok(frame) => frame,
            // A malformed/truncated frame affects only this connection.
            Err(_) => return,
        };
        let request = match decode_request(&frame) {
            Ok(request) => request,
            Err(error) => {
                let _ = send_response(
                    &mut framed,
                    &IpcResponse::Error {
                        message: format!("{error:#}"),
                    },
                )
                .await;
                return;
            }
        };
        if let IpcRequest::FollowLogs { id } = request {
            if let Err(error) = stream_logs(&mut framed, &scheduler, id, &mut shutdown_rx).await {
                let _ = send_response(
                    &mut framed,
                    &IpcResponse::Error {
                        message: format!("{error:#}"),
                    },
                )
                .await;
            }
            return;
        }
        let stop = matches!(request, IpcRequest::Stop);
        let response = match request {
            IpcRequest::Status => match scheduler.service_status() {
                Ok(status) => IpcResponse::Status(status),
                Err(error) => IpcResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            IpcRequest::Stop => {
                // Close scheduler intake before waiting for active cleanup.
                // The endpoint remains alive until this handler replies and
                // the accept loop drains, allowing the client to observe a
                // completed stop rather than a fire-and-forget request.
                scheduler.begin_shutdown();
                let _ = shutdown_tx.send(true);
                match scheduler.stop_active().await {
                    Ok(()) => IpcResponse::Ack,
                    Err(error) => IpcResponse::Error {
                        message: format!("{error:#}"),
                    },
                }
            }
            IpcRequest::Commit { id } => match scheduler.handle_commit(id, &wake_tx) {
                Ok(_) => IpcResponse::Ack,
                Err(error) => IpcResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            IpcRequest::CommitAll => match scheduler.handle_commit_all(&wake_tx) {
                Ok(jobs) => IpcResponse::JobCount { count: jobs.len() },
                Err(error) => IpcResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            IpcRequest::Cancel { id } => match scheduler.handle_cancel(id).await {
                Ok(_) => IpcResponse::Ack,
                Err(error) => IpcResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            IpcRequest::LockQueue => match scheduler.handle_lock_queue() {
                Ok(()) => IpcResponse::Ack,
                Err(error) => IpcResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            IpcRequest::UnlockQueue => match scheduler.handle_unlock_queue(&wake_tx) {
                Ok(()) => IpcResponse::Ack,
                Err(error) => IpcResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            IpcRequest::MoveQueued { id, target_order } => {
                match scheduler.handle_move_queued(id, target_order) {
                    Ok(jobs) => IpcResponse::QueuedJobs { jobs },
                    Err(error) if is_stale_move_error(&error) => IpcResponse::StaleQueueMove {
                        message: format!("{error:#}"),
                    },
                    Err(error) => IpcResponse::Error {
                        message: format!("{error:#}"),
                    },
                }
            }
            IpcRequest::FollowLogs { .. } => unreachable!(),
        };
        if send_response(&mut framed, &response).await.is_err() {
            return;
        }
        if stop {
            let _ = shutdown_tx.send(true);
            return;
        }
    }
}

fn is_stale_move_error(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<StoreError>() {
        return matches!(
            error,
            StoreError::NotFound { .. }
                | StoreError::InvalidQueueOrder { .. }
                | StoreError::InvalidTransition { action: "move", .. }
        );
    }
    false
}

async fn stream_logs<S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    scheduler: &Scheduler,
    id: uuid::Uuid,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if *shutdown_rx.borrow() {
        anyhow::bail!("scheduler is shutting down; log follow cancelled");
    }
    let job = scheduler.job_exists(id)?;
    let (stdout, stderr) = scheduler.log_paths(id);
    let mut receiver = scheduler.log_receiver(id);
    // A commit response only acknowledges persistence. Give the scheduler a
    // short window to claim the queued job and create its run directory before
    // a follower attempts its initial read.
    if receiver.is_none()
        && matches!(
            job.state,
            crate::JobState::Queued | crate::JobState::Starting | crate::JobState::Running
        )
    {
        // Do not impose a wall-clock limit: a queued job may legitimately wait
        // behind arbitrarily long work. Stop waiting only when its durable
        // state becomes terminal (or it is canceled before starting).
        loop {
            if *shutdown_rx.borrow() {
                anyhow::bail!("scheduler is shutting down; log follow cancelled");
            }
            if let Some(found) = scheduler.log_receiver(id) {
                receiver = Some(found);
                break;
            }
            let state = scheduler.job_exists(id)?.state;
            if matches!(
                state,
                crate::JobState::Succeeded
                    | crate::JobState::Failed
                    | crate::JobState::Cancelled
                    | crate::JobState::Lost
            ) {
                break;
            }
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        anyhow::bail!("scheduler is shutting down; log follow cancelled");
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            }
        }
    }
    let mut delivered = [0_u64, 0_u64];
    for (index, path) in [stdout.as_path(), stderr.as_path()].iter().enumerate() {
        match tokio::fs::read(path).await {
            Ok(bytes) if !bytes.is_empty() => {
                delivered[index] = bytes.len() as u64;
                send_response(
                    framed,
                    &IpcResponse::LogChunk {
                        stream: if index == 0 {
                            LogStream::Stdout
                        } else {
                            LogStream::Stderr
                        },
                        bytes,
                    },
                )
                .await?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if receiver.is_none() {
                    anyhow::bail!("log file {} does not exist", path.display());
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    let Some(mut receiver) = receiver.take() else {
        send_response(framed, &IpcResponse::LogEnd).await?;
        return Ok(());
    };
    loop {
        if *shutdown_rx.borrow() {
            anyhow::bail!("scheduler is shutting down; log follow cancelled");
        }
        let message = tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    anyhow::bail!("scheduler is shutting down; log follow cancelled");
                }
                continue;
            }
            message = receiver.recv() => message,
        };
        match message {
            Ok(LogMessage::End) => {
                // The final flush can race the End notification; read once
                // more so followers never miss trailing bytes.
                for (index, path) in [stdout.as_path(), stderr.as_path()].iter().enumerate() {
                    if let Ok(bytes) = tokio::fs::read(path).await {
                        let start = delivered[index] as usize;
                        if bytes.len() > start {
                            send_response(
                                framed,
                                &IpcResponse::LogChunk {
                                    stream: if index == 0 {
                                        LogStream::Stdout
                                    } else {
                                        LogStream::Stderr
                                    },
                                    bytes: bytes[start..].to_vec(),
                                },
                            )
                            .await?;
                        }
                    }
                }
                send_response(framed, &IpcResponse::LogEnd).await?;
                return Ok(());
            }
            Ok(LogMessage::Chunk(event)) => {
                let index = if event.stream == LogStream::Stdout {
                    0
                } else {
                    1
                };
                let start = delivered[index].max(event.offset);
                let skip = start.saturating_sub(event.offset) as usize;
                if skip < event.bytes.len() {
                    send_response(
                        framed,
                        &IpcResponse::LogChunk {
                            stream: event.stream,
                            bytes: event.bytes[skip..].to_vec(),
                        },
                    )
                    .await?;
                    delivered[index] = event.offset + event.bytes.len() as u64;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                for (index, path) in [stdout.as_path(), stderr.as_path()].iter().enumerate() {
                    if let Ok(bytes) = tokio::fs::read(path).await {
                        let start = delivered[index] as usize;
                        if bytes.len() > start {
                            send_response(
                                framed,
                                &IpcResponse::LogChunk {
                                    stream: if index == 0 {
                                        LogStream::Stdout
                                    } else {
                                        LogStream::Stderr
                                    },
                                    bytes: bytes[start..].to_vec(),
                                },
                            )
                            .await?;
                            delivered[index] = bytes.len() as u64;
                        }
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                send_response(framed, &IpcResponse::LogEnd).await?;
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JobState, NewJob};
    use std::path::PathBuf;

    #[tokio::test]
    async fn queued_log_follow_exits_if_shutdown_is_already_set() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let id = store
            .create_job(NewJob {
                name: "queued".into(),
                user: "test".into(),
                cwd: PathBuf::from("."),
                command: vec!["echo".into(), "queued".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();
        let scheduler = Scheduler::new(paths, store);
        let (stream, _peer) = tokio::io::duplex(1024);
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let (_shutdown, mut shutdown_rx) = watch::channel(true);

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            stream_logs(&mut framed, &scheduler, id, &mut shutdown_rx),
        )
        .await
        .expect("queued follower should stop promptly")
        .unwrap_err();
        assert!(result.to_string().contains("shutting down"));
        assert_eq!(scheduler.job_exists(id).unwrap().state, JobState::Queued);
    }
}
