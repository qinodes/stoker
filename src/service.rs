//! Singleton service bootstrap and IPC request handling.

mod dispatch;
mod error_mapping;
mod log_stream;
mod transport;

use std::fs::{File, OpenOptions};
use std::sync::Arc;

use anyhow::Context;
use fs2::FileExt;
use tokio::sync::watch;
#[cfg(test)]
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::domain::JobState;
use crate::ipc::ServiceStatus;
#[cfg(test)]
use crate::ipc::{IpcRequest, IpcResponse, LogStream};
use crate::scheduler::Scheduler;
use crate::{StokerPaths, Store};

#[cfg(test)]
use dispatch::handle_client;
#[cfg(test)]
use log_stream::stream_logs;

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
        transport::run(self, scheduler, wake_tx, scheduler_wake).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{decode_response, encode_request};
    use crate::process::{ManagedProcess, ProcessController, ProcessSpec};
    use crate::scheduler::{LogEvent, LogMessage, OutputStream};
    use crate::{JobState, NewJob};
    use async_trait::async_trait;
    use futures_util::{SinkExt, StreamExt};
    use std::io;
    use std::path::PathBuf;
    use std::process::Command;
    use uuid::Uuid;

    fn paths(root: &std::path::Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    fn scheduler_fixture() -> (tempfile::TempDir, Arc<Store>, Arc<Scheduler>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let scheduler = Arc::new(Scheduler::new(paths, Arc::clone(&store)));
        (directory, store, scheduler)
    }

    struct StreamingProcessController;

    struct StreamingProcess {
        stdout: PathBuf,
        stderr: PathBuf,
    }

    #[async_trait]
    impl ProcessController for StreamingProcessController {
        async fn spawn(&self, spec: ProcessSpec) -> io::Result<Box<dyn ManagedProcess>> {
            Ok(Box::new(StreamingProcess {
                stdout: spec.stdout_log,
                stderr: spec.stderr_log,
            }))
        }
    }

    #[async_trait]
    impl ManagedProcess for StreamingProcess {
        fn pid(&self) -> u32 {
            5252
        }

        async fn wait(self: Box<Self>) -> io::Result<std::process::ExitStatus> {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            tokio::fs::write(&self.stdout, b"out").await?;
            tokio::fs::write(&self.stderr, b"err").await?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            tokio::fs::write(&self.stdout, b"out-tail").await?;
            tokio::fs::write(&self.stderr, b"err-tail").await?;
            let mut command = if cfg!(windows) {
                Command::new("cmd.exe")
            } else {
                Command::new("true")
            };
            if cfg!(windows) {
                command.args(["/C", "exit", "0"]);
            }
            command.status()
        }

        async fn terminate_tree(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    async fn request_response(scheduler: Arc<Scheduler>, request: IpcRequest) -> IpcResponse {
        let (server_stream, client_stream) = tokio::io::duplex(8192);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (wake_tx, _) = watch::channel(0_u64);
        let server_task = tokio::spawn(handle_client(
            server_stream,
            scheduler,
            wake_tx,
            shutdown_tx,
            shutdown_rx,
        ));
        let mut framed = Framed::new(client_stream, LengthDelimitedCodec::new());
        framed
            .send(encode_request(&request).unwrap().into())
            .await
            .unwrap();
        let response = decode_response(&framed.next().await.unwrap().unwrap()).unwrap();
        drop(framed);
        server_task.await.unwrap();
        response
    }

    async fn raw_response(scheduler: Arc<Scheduler>, frame: &[u8]) -> IpcResponse {
        let (server_stream, client_stream) = tokio::io::duplex(8192);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (wake_tx, _) = watch::channel(0_u64);
        let server_task = tokio::spawn(handle_client(
            server_stream,
            scheduler,
            wake_tx,
            shutdown_tx,
            shutdown_rx,
        ));
        let mut framed = Framed::new(client_stream, LengthDelimitedCodec::new());
        framed.send(frame.to_vec().into()).await.unwrap();
        let response = decode_response(&framed.next().await.unwrap().unwrap()).unwrap();
        drop(framed);
        server_task.await.unwrap();
        response
    }

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
                description: None,
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

    #[tokio::test]
    async fn client_handler_dispatches_requests_and_reports_failures() {
        let (_directory, store, scheduler) = scheduler_fixture();
        let status = request_response(Arc::clone(&scheduler), IpcRequest::Status).await;
        assert!(matches!(
            status,
            IpcResponse::Status(ServiceStatus {
                queued_jobs: 0,
                queue_locked: false,
                active_job: None,
                ..
            })
        ));

        let missing = Uuid::nil();
        let response =
            request_response(Arc::clone(&scheduler), IpcRequest::Commit { id: missing }).await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::NotFound
        ));

        assert_eq!(
            request_response(Arc::clone(&scheduler), IpcRequest::CommitAll).await,
            IpcResponse::Jobs { jobs: Vec::new() }
        );
        assert_eq!(
            request_response(
                Arc::clone(&scheduler),
                IpcRequest::CommitMany { ids: Vec::new() },
            )
            .await,
            IpcResponse::Jobs { jobs: Vec::new() }
        );
        assert_eq!(
            request_response(
                Arc::clone(&scheduler),
                IpcRequest::CommitUser {
                    user: "alice".into(),
                },
            )
            .await,
            IpcResponse::Jobs { jobs: Vec::new() }
        );
        let response =
            request_response(Arc::clone(&scheduler), IpcRequest::Cancel { id: missing }).await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::NotFound
        ));

        assert_eq!(
            request_response(Arc::clone(&scheduler), IpcRequest::LockQueue).await,
            IpcResponse::Queue {
                jobs: Vec::new(),
                locked: true,
            }
        );
        let response = request_response(Arc::clone(&scheduler), IpcRequest::CommitAll).await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::QueueLocked
        ));
        let response = request_response(
            Arc::clone(&scheduler),
            IpcRequest::CommitMany { ids: Vec::new() },
        )
        .await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::QueueLocked
        ));
        let response = request_response(
            Arc::clone(&scheduler),
            IpcRequest::CommitUser {
                user: "alice".into(),
            },
        )
        .await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::QueueLocked
        ));
        let response = request_response(
            Arc::clone(&scheduler),
            IpcRequest::MoveQueued {
                id: missing,
                target_order: 1,
            },
        )
        .await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::StaleQueue
        ));
        assert_eq!(
            request_response(Arc::clone(&scheduler), IpcRequest::UnlockQueue).await,
            IpcResponse::Queue {
                jobs: Vec::new(),
                locked: false,
            }
        );
        let response = request_response(
            Arc::clone(&scheduler),
            IpcRequest::MoveQueued {
                id: missing,
                target_order: 1,
            },
        )
        .await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::QueueUnlocked
        ));
        assert_eq!(
            request_response(Arc::clone(&scheduler), IpcRequest::Stop).await,
            IpcResponse::Ack
        );

        let invalid = raw_response(Arc::clone(&scheduler), b"not-json").await;
        assert!(matches!(
            invalid,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::InvalidRequest
        ));

        let id = store
            .create_job(NewJob {
                name: "draft".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        let response = request_response(scheduler, IpcRequest::FollowLogs { id }).await;
        assert!(matches!(
            response,
            IpcResponse::Error(error) if error.code == crate::ipc::IpcErrorCode::InvalidState
        ));
    }

    #[tokio::test]
    async fn stream_logs_sends_existing_stdout_stderr_and_log_end() {
        let (directory, store, scheduler) = scheduler_fixture();
        let id = store
            .create_job(NewJob {
                name: "finished".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.cancel_not_started(id).unwrap();
        let run_dir = directory.path().join("runs").join(id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(run_dir.join("stdout.log"), b"out").unwrap();
        std::fs::write(run_dir.join("stderr.log"), b"err").unwrap();

        let (server_stream, client_stream) = tokio::io::duplex(8192);
        let mut server_framed = Framed::new(server_stream, LengthDelimitedCodec::new());
        let mut client_framed = Framed::new(client_stream, LengthDelimitedCodec::new());
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(async move {
            stream_logs(&mut server_framed, &scheduler, id, &mut shutdown_rx).await
        });

        let mut responses = Vec::new();
        loop {
            let frame = client_framed.next().await.unwrap().unwrap();
            let response = decode_response(&frame).unwrap();
            let finished = response == IpcResponse::LogEnd;
            responses.push(response);
            if finished {
                break;
            }
        }
        server_task.await.unwrap().unwrap();
        assert!(responses.contains(&IpcResponse::LogChunk {
            stream: LogStream::Stdout,
            bytes: b"out".to_vec(),
        }));
        assert!(responses.contains(&IpcResponse::LogChunk {
            stream: LogStream::Stderr,
            bytes: b"err".to_vec(),
        }));
    }

    #[tokio::test]
    async fn stream_logs_forwards_live_chunks_and_flushes_trailing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let id = store
            .create_job(NewJob {
                name: "running".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();
        let scheduler = Arc::new(Scheduler::with_controller(
            paths,
            Arc::clone(&store),
            Arc::new(StreamingProcessController),
        ));
        let mut process = StreamingProcess {
            stdout: PathBuf::new(),
            stderr: PathBuf::new(),
        };
        process.terminate_tree().await.unwrap();
        let (_wake_tx, wake_rx) = watch::channel(0_u64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));
        for _ in 0..20 {
            if scheduler.log_receiver(id).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(scheduler.log_receiver(id).is_some());
        let (server_stream, client_stream) = tokio::io::duplex(8192);
        let mut server_framed = Framed::new(server_stream, LengthDelimitedCodec::new());
        let mut client_framed = Framed::new(client_stream, LengthDelimitedCodec::new());
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let stream_scheduler = Arc::clone(&scheduler);
        let task = tokio::spawn(async move {
            stream_logs(&mut server_framed, &stream_scheduler, id, &mut shutdown_rx).await
        });

        let mut responses = Vec::new();
        loop {
            let frame = client_framed.next().await.unwrap().unwrap();
            let response = decode_response(&frame).unwrap();
            let finished = response == IpcResponse::LogEnd;
            responses.push(response);
            if finished {
                break;
            }
        }
        task.await.unwrap().unwrap();
        shutdown_tx.send(true).unwrap();
        scheduler_task.await.unwrap().unwrap();
        assert!(responses.contains(&IpcResponse::LogChunk {
            stream: LogStream::Stdout,
            bytes: b"out".to_vec(),
        }));
        assert!(responses.contains(&IpcResponse::LogChunk {
            stream: LogStream::Stderr,
            bytes: b"err".to_vec(),
        }));
        assert!(responses.contains(&IpcResponse::LogChunk {
            stream: LogStream::Stdout,
            bytes: b"-tail".to_vec(),
        }));
        assert!(responses.contains(&IpcResponse::LogChunk {
            stream: LogStream::Stderr,
            bytes: b"-tail".to_vec(),
        }));
    }

    #[tokio::test]
    async fn lagged_log_receiver_recovers_from_durable_files() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.database).unwrap());
        let id = store
            .create_job(NewJob {
                name: "lagged".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.commit_job(id).unwrap();
        store.claim_next().unwrap();
        store.set_running(id, 4242).unwrap();
        let scheduler = Arc::new(Scheduler::new(paths.clone(), store));
        let sender = scheduler.install_test_log_sender(id, 2);
        let run_dir = paths.runs.join(id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let stdout = run_dir.join("stdout.log");
        std::fs::write(&stdout, b"initial").unwrap();
        std::fs::write(run_dir.join("stderr.log"), b"").unwrap();

        let (server_stream, client_stream) = tokio::io::duplex(8192);
        let mut server_framed = Framed::new(server_stream, LengthDelimitedCodec::new());
        let mut client_framed = Framed::new(client_stream, LengthDelimitedCodec::new());
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let stream_scheduler = Arc::clone(&scheduler);
        let task = tokio::spawn(async move {
            stream_logs(&mut server_framed, &stream_scheduler, id, &mut shutdown_rx).await
        });
        let initial = decode_response(&client_framed.next().await.unwrap().unwrap()).unwrap();
        assert_eq!(
            initial,
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: b"initial".to_vec(),
            }
        );
        assert_eq!(sender.receiver_count(), 1);

        std::fs::write(&stdout, b"initialcomplete").unwrap();
        for byte in b"abcd" {
            let _ = sender.send(LogMessage::Chunk(LogEvent {
                stream: OutputStream::Stdout,
                offset: 0,
                bytes: vec![*byte],
            }));
        }
        let _ = sender.send(LogMessage::End);

        let first = decode_response(&client_framed.next().await.unwrap().unwrap()).unwrap();
        let second = decode_response(&client_framed.next().await.unwrap().unwrap()).unwrap();
        assert_eq!(
            first,
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: b"complete".to_vec(),
            }
        );
        assert_eq!(second, IpcResponse::LogEnd);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stream_logs_reports_missing_files_when_no_live_log_sender_exists() {
        let (_directory, store, scheduler) = scheduler_fixture();
        let id = store
            .create_job(NewJob {
                name: "finished".into(),
                user: "test".into(),
                description: None,
                cwd: PathBuf::from("."),
                command: vec!["echo".into()],
            })
            .unwrap();
        store.cancel_not_started(id).unwrap();
        let (server_stream, _client_stream) = tokio::io::duplex(8192);
        let mut framed = Framed::new(server_stream, LengthDelimitedCodec::new());
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let error = stream_logs(&mut framed, &scheduler, id, &mut shutdown_rx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn queued_log_follow_can_be_cancelled_while_waiting_for_scheduler_start() {
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
        let (server_stream, _client_stream) = tokio::io::duplex(8192);
        let mut framed = Framed::new(server_stream, LengthDelimitedCodec::new());
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            stream_logs(&mut framed, &scheduler, id, &mut shutdown_rx).await
        });
        tokio::task::yield_now().await;
        shutdown_tx.send(true).unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("shutting down"));
    }

    #[tokio::test]
    async fn queued_log_follow_reports_missing_logs_after_job_becomes_terminal() {
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

        let (server_stream, _client_stream) = tokio::io::duplex(8192);
        let mut framed = Framed::new(server_stream, LengthDelimitedCodec::new());
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            stream_logs(&mut framed, &scheduler, id, &mut shutdown_rx).await
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        store.cancel_not_started(id).unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("terminal queued follower should finish")
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn service_run_accepts_status_and_stop_requests_then_cleans_up_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let service = Service::new(paths.clone()).unwrap();
        let service_task = tokio::spawn(service.run());
        let client = crate::ServiceClient::new(paths.clone());

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if client.status().await.is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("service should accept status requests");

        client.stop().await.unwrap();
        service_task.await.unwrap().unwrap();
        assert!(!paths.endpoint.exists());
    }

    #[test]
    fn service_status_reports_queued_and_active_jobs() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let store = Store::open(&paths.database).unwrap();
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
        let service = Service::new(paths.clone()).unwrap();
        assert_eq!(service.status().unwrap().queued_jobs, 1);
        store.claim_next().unwrap();
        assert_eq!(service.status().unwrap().active_job, Some(id));
    }

    #[test]
    fn service_lock_rejects_a_second_service_instance() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let _first = Service::new(paths.clone()).unwrap();
        let error = match Service::new(paths) {
            Ok(_) => panic!("second service unexpectedly acquired the lock"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("already running"));
    }
}
