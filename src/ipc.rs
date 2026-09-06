//! Versioned local IPC protocol and client.

use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use anyhow::Context;
use fs2::FileExt;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;

use crate::{Job, StokerPaths};

pub const IPC_VERSION: u16 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcRequest {
    Status,
    Stop,
    Commit { id: Uuid },
    CommitMany { ids: Vec<Uuid> },
    CommitAll,
    CommitUser { user: String },
    Cancel { id: Uuid },
    FollowLogs { id: Uuid },
    LockQueue,
    UnlockQueue,
    MoveQueued { id: Uuid, target_order: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcResponse {
    Ack,
    JobCount { count: usize },
    QueuedJobs { jobs: Vec<Job> },
    StaleQueueMove { message: String },
    Status(ServiceStatus),
    LogChunk { stream: LogStream, bytes: Vec<u8> },
    LogEnd,
    Error { message: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub pid: u32,
    pub active_job: Option<Uuid>,
    pub queued_jobs: usize,
    pub queue_locked: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("scheduler service unavailable: {0}")]
pub struct ServiceUnavailable(#[source] std::io::Error);

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct StaleQueueMoveError {
    message: String,
}

impl StaleQueueMoveError {
    fn new(message: String) -> Self {
        Self { message }
    }
}

pub fn is_service_unavailable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ServiceUnavailable>().is_some()
}

#[derive(Debug, Serialize, Deserialize)]
struct VersionedRequest {
    version: u16,
    request: IpcRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct VersionedResponse {
    version: u16,
    response: IpcResponse,
}

pub(crate) fn encode_request(request: &IpcRequest) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&VersionedRequest {
        version: IPC_VERSION,
        request: request.clone(),
    })?)
}

pub(crate) fn decode_request(frame: &[u8]) -> anyhow::Result<IpcRequest> {
    let message: VersionedRequest = serde_json::from_slice(frame)?;
    if message.version != IPC_VERSION {
        anyhow::bail!("unsupported IPC protocol version {}", message.version);
    }
    Ok(message.request)
}

pub(crate) fn encode_response(response: &IpcResponse) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&VersionedResponse {
        version: IPC_VERSION,
        response: response.clone(),
    })?)
}

pub(crate) fn decode_response(frame: &[u8]) -> anyhow::Result<IpcResponse> {
    let message: VersionedResponse = serde_json::from_slice(frame)?;
    if message.version != IPC_VERSION {
        anyhow::bail!("unsupported IPC protocol version {}", message.version);
    }
    Ok(message.response)
}

#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

#[derive(Debug, Clone)]
pub struct ServiceClient {
    paths: StokerPaths,
    timeout: Duration,
}

impl ServiceClient {
    pub fn new(paths: StokerPaths) -> Self {
        Self {
            paths,
            timeout: Duration::from_secs(2),
        }
    }

    pub async fn status(&self) -> anyhow::Result<ServiceStatus> {
        match self.request(IpcRequest::Status).await? {
            IpcResponse::Status(status) => Ok(status),
            IpcResponse::Ack
            | IpcResponse::JobCount { .. }
            | IpcResponse::QueuedJobs { .. }
            | IpcResponse::StaleQueueMove { .. } => {
                anyhow::bail!("service returned an invalid status response")
            }
            IpcResponse::LogChunk { .. } | IpcResponse::LogEnd => {
                anyhow::bail!("service returned an invalid status response")
            }
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
        }
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::Stop).await? {
            IpcResponse::Ack => {
                // The response is written before the service begins teardown;
                // wait until its endpoint disappears so callers can safely
                // start another instance immediately.
                let deadline = tokio::time::Instant::now() + self.timeout;
                loop {
                    if tokio::time::Instant::now() >= deadline {
                        anyhow::bail!(
                            "scheduler service did not stop within {} seconds",
                            self.timeout.as_secs()
                        );
                    }
                    if !self.endpoint_is_reachable().await && self.service_lock_is_available() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(())
            }
            IpcResponse::Status(_)
            | IpcResponse::JobCount { .. }
            | IpcResponse::QueuedJobs { .. }
            | IpcResponse::StaleQueueMove { .. } => {
                anyhow::bail!("service returned an invalid stop response")
            }
            IpcResponse::LogChunk { .. } | IpcResponse::LogEnd => {
                anyhow::bail!("service returned an invalid stop response")
            }
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
        }
    }

    pub async fn commit(&self, id: Uuid) -> anyhow::Result<()> {
        match self.request(IpcRequest::Commit { id }).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid commit response"),
        }
    }

    pub async fn commit_many(&self, ids: Vec<Uuid>) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitMany { ids }).await? {
            IpcResponse::JobCount { count } => Ok(count),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid commit response"),
        }
    }

    pub async fn commit_all(&self) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitAll).await? {
            IpcResponse::JobCount { count } => Ok(count),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid commit response"),
        }
    }

    pub async fn commit_user(&self, user: String) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitUser { user }).await? {
            IpcResponse::JobCount { count } => Ok(count),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid commit response"),
        }
    }

    pub async fn cancel(&self, id: Uuid) -> anyhow::Result<()> {
        match self.request(IpcRequest::Cancel { id }).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid cancel response"),
        }
    }

    pub async fn lock_queue(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::LockQueue).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid lock queue response"),
        }
    }

    pub async fn unlock_queue(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::UnlockQueue).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid unlock queue response"),
        }
    }

    pub async fn move_queued(&self, id: Uuid, target_order: usize) -> anyhow::Result<Vec<Job>> {
        match self
            .request(IpcRequest::MoveQueued { id, target_order })
            .await?
        {
            IpcResponse::QueuedJobs { jobs } => Ok(jobs),
            IpcResponse::StaleQueueMove { message } => {
                Err(StaleQueueMoveError::new(message).into())
            }
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid move queued response"),
        }
    }

    /// Follow a job's output and write stdout/stderr chunks to their matching
    /// local streams. The call returns only after the service sends LogEnd.
    pub async fn follow_logs(&self, id: Uuid) -> anyhow::Result<()> {
        let stream = tokio::time::timeout(self.timeout, connect_with_retry(&self.paths))
            .await
            .map_err(|_| {
                anyhow::Error::new(ServiceUnavailable(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "connection timed out",
                )))
            })?
            .map_err(|error| anyhow::Error::new(ServiceUnavailable(error)))?;
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let payload = encode_request(&IpcRequest::FollowLogs { id })?;
        tokio::time::timeout(self.timeout, framed.send(payload.into()))
            .await
            .map_err(|_| anyhow::anyhow!("send timed out"))??;
        while let Some(frame) = framed.next().await {
            let response = decode_response(&frame?)?;
            match response {
                IpcResponse::LogChunk { stream, bytes } => match stream {
                    LogStream::Stdout => std::io::stdout().write_all(&bytes)?,
                    LogStream::Stderr => std::io::stderr().write_all(&bytes)?,
                },
                IpcResponse::LogEnd => return Ok(()),
                IpcResponse::Error { message } => anyhow::bail!("{message}"),
                _ => anyhow::bail!("service returned an invalid log response"),
            }
        }
        anyhow::bail!("scheduler closed the log stream")
    }

    pub(crate) async fn request(&self, request: IpcRequest) -> anyhow::Result<IpcResponse> {
        let stream = tokio::time::timeout(self.timeout, connect_with_retry(&self.paths))
            .await
            .map_err(|_| {
                anyhow::Error::new(ServiceUnavailable(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "connection timed out",
                )))
            })?
            .map_err(|error| anyhow::Error::new(ServiceUnavailable(error)))?;
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let payload = encode_request(&request)?;
        tokio::time::timeout(self.timeout, framed.send(payload.into()))
            .await
            .map_err(|_| {
                anyhow::Error::new(ServiceUnavailable(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "send timed out",
                )))
            })?
            .map_err(|error| anyhow::Error::new(ServiceUnavailable(error)))?;
        let frame = tokio::time::timeout(self.timeout, framed.next())
            .await
            .context("wait for scheduler response")?
            .ok_or_else(|| anyhow::anyhow!("scheduler closed the IPC connection"))??;
        decode_response(&frame)
    }

    async fn endpoint_is_reachable(&self) -> bool {
        match tokio::time::timeout(Duration::from_millis(100), connect(&self.paths)).await {
            Ok(Ok(_)) => true,
            Ok(Err(_)) => false,
            // A connect timeout means the endpoint may still be alive; keep
            // waiting until the overall stop deadline rather than claiming it
            // has disappeared.
            Err(_) => true,
        }
    }

    fn service_lock_is_available(&self) -> bool {
        service_lock_is_available(&self.paths)
    }
}

fn service_lock_is_available(paths: &StokerPaths) -> bool {
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)
    else {
        return false;
    };
    if lock.try_lock_exclusive().is_err() {
        return false;
    }
    let _ = FileExt::unlock(&lock);
    true
}

pub(crate) async fn send_response<S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    response: &IpcResponse,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    framed.send(encode_response(response)?.into()).await?;
    Ok(())
}

#[cfg(unix)]
async fn connect(paths: &StokerPaths) -> std::io::Result<IpcStream> {
    tokio::net::UnixStream::connect(&paths.endpoint).await
}

#[cfg(windows)]
async fn connect(paths: &StokerPaths) -> std::io::Result<IpcStream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    ClientOptions::new().open(paths.ipc_endpoint())
}

async fn connect_with_retry(paths: &StokerPaths) -> std::io::Result<IpcStream> {
    #[cfg(windows)]
    {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            match connect(paths).await {
                Ok(client) => return Ok(client),
                Err(error)
                    if is_retryable_pipe_connect_error(paths, &error)
                        && tokio::time::Instant::now() < deadline =>
                {
                    // The service creates a fresh named-pipe instance after
                    // each client disconnects. Retry the brief gap, including
                    // ERROR_PIPE_BUSY when another client is using the only
                    // currently available instance.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
    #[cfg(unix)]
    {
        connect(paths).await
    }
}

#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

#[cfg(windows)]
fn is_retryable_pipe_connect_error(paths: &StokerPaths, error: &std::io::Error) -> bool {
    (error.kind() == std::io::ErrorKind::NotFound && !service_lock_is_available(paths))
        || error.raw_os_error() == Some(ERROR_PIPE_BUSY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Job, JobState};
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::time::{Duration, Instant};

    #[test]
    fn frames_include_protocol_version_and_round_trip() {
        let id = Uuid::nil();
        for request in [
            IpcRequest::Status,
            IpcRequest::Commit { id },
            IpcRequest::CommitMany { ids: vec![id] },
            IpcRequest::CommitAll,
            IpcRequest::CommitUser {
                user: "alice".to_owned(),
            },
            IpcRequest::LockQueue,
            IpcRequest::UnlockQueue,
            IpcRequest::MoveQueued {
                id,
                target_order: 2,
            },
        ] {
            let encoded = encode_request(&request).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(value["version"], IPC_VERSION);
            assert_eq!(decode_request(&encoded).unwrap(), request);
        }

        let job = Job {
            id,
            name: "queued-job".to_owned(),
            user: "alice".to_owned(),
            cwd: PathBuf::from("/tmp/workspace"),
            command: vec!["echo".to_owned(), "hello".to_owned()],
            command_line: Some("echo hello".to_owned()),
            state: JobState::Queued,
            queue_order: Some(1),
            created_at: chrono::Utc::now(),
            committed_at: Some(chrono::Utc::now()),
            started_at: None,
            finished_at: None,
            exit_code: None,
            pid: None,
            failure_detail: None,
        };
        let queued_jobs = IpcResponse::QueuedJobs { jobs: vec![job] };
        let encoded = encode_response(&queued_jobs).unwrap();
        assert_eq!(decode_response(&encoded).unwrap(), queued_jobs);

        let stale_move = IpcResponse::StaleQueueMove {
            message: "selected job was removed".to_owned(),
        };
        let encoded = encode_response(&stale_move).unwrap();
        assert_eq!(decode_response(&encoded).unwrap(), stale_move);

        let response = IpcResponse::Status(ServiceStatus {
            pid: 42,
            active_job: None,
            queued_jobs: 3,
            queue_locked: true,
        });
        let encoded = encode_response(&response).unwrap();
        assert_eq!(decode_response(&encoded).unwrap(), response);
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        let frame = serde_json::json!({
            "version": 1,
            "request": "Status"
        });
        let error = decode_request(&serde_json::to_vec(&frame).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported IPC protocol version")
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn missing_service_returns_without_waiting_for_retry_window() {
        let root = tempfile::tempdir().unwrap();
        let paths = StokerPaths {
            database: root.path().join("stoker.db"),
            runs: root.path().join("runs"),
            lock: root.path().join("stoker.lock"),
            endpoint: root.path().join("stoker.sock"),
            root: root.path().to_path_buf(),
        };
        let started = Instant::now();

        let error = connect_with_retry(&paths).await.unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "missing service took {:?} to report unavailable",
            started.elapsed()
        );
    }
}

#[cfg(all(test, unix))]
mod unix_client_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::future::Future;
    use std::path::Path;
    use tokio::net::UnixListener;
    use tokio::task::JoinHandle;
    use tokio_util::codec::{Framed, LengthDelimitedCodec};

    fn paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    async fn client_with_responses(
        responses: Vec<IpcResponse>,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let listener = UnixListener::bind(&paths.endpoint).unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            for response in responses {
                framed
                    .send(encode_response(&response).unwrap().into())
                    .await
                    .unwrap();
            }
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn assert_response_error<T, F, Fut>(response: IpcResponse, operation: F, expected: &str)
    where
        F: FnOnce(ServiceClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (_directory, client, task) = client_with_responses(vec![response]).await;
        let error = operation(client)
            .await
            .err()
            .expect("operation unexpectedly succeeded");
        assert!(error.to_string().contains(expected), "{error:#}");
        task.await.unwrap();
    }

    fn sample_status() -> ServiceStatus {
        ServiceStatus {
            pid: 42,
            active_job: None,
            queued_jobs: 0,
            queue_locked: false,
        }
    }

    #[tokio::test]
    async fn client_methods_validate_response_variants() {
        let (_directory, client, task) =
            client_with_responses(vec![IpcResponse::Status(sample_status())]).await;
        assert_eq!(client.status().await.unwrap(), sample_status());
        task.await.unwrap();

        for response in [
            IpcResponse::Ack,
            IpcResponse::JobCount { count: 1 },
            IpcResponse::QueuedJobs { jobs: Vec::new() },
            IpcResponse::StaleQueueMove {
                message: "stale".into(),
            },
        ] {
            assert_response_error(
                response,
                |client| async move { client.status().await },
                "invalid status",
            )
            .await;
        }
        for response in [
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ] {
            assert_response_error(
                response,
                |client| async move { client.status().await },
                "invalid status",
            )
            .await;
        }
        assert_response_error(
            IpcResponse::Error {
                message: "status failed".into(),
            },
            |client| async move { client.status().await },
            "status failed",
        )
        .await;

        let id = Uuid::nil();
        for (operation, expected) in [
            ("commit", "invalid commit"),
            ("cancel", "invalid cancel"),
            ("lock queue", "invalid lock queue"),
            ("unlock queue", "invalid unlock queue"),
        ] {
            let response = IpcResponse::Status(sample_status());
            match operation {
                "commit" => {
                    assert_response_error(
                        response,
                        |client| async move { client.commit(id).await },
                        expected,
                    )
                    .await
                }
                "cancel" => {
                    assert_response_error(
                        response,
                        |client| async move { client.cancel(id).await },
                        expected,
                    )
                    .await
                }
                "lock queue" => {
                    assert_response_error(
                        response,
                        |client| async move { client.lock_queue().await },
                        expected,
                    )
                    .await
                }
                "unlock queue" => {
                    assert_response_error(
                        response,
                        |client| async move { client.unlock_queue().await },
                        expected,
                    )
                    .await
                }
                _ => unreachable!(),
            }
        }

        assert_response_error(
            IpcResponse::Error {
                message: "commit failed".into(),
            },
            |client| async move { client.commit(id).await },
            "commit failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Error {
                message: "commit many failed".into(),
            },
            |client| async move { client.commit_many(vec![id]).await },
            "commit many failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_many(vec![id]).await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            IpcResponse::Error {
                message: "commit user failed".into(),
            },
            |client| async move { client.commit_user("alice".to_owned()).await },
            "commit user failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_user("alice".to_owned()).await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(sample_status()),
            |client| async move { client.commit_all().await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            IpcResponse::Error {
                message: "cancel failed".into(),
            },
            |client| async move { client.cancel(id).await },
            "cancel failed",
        )
        .await;

        let (_directory, client, task) =
            client_with_responses(vec![IpcResponse::JobCount { count: 3 }]).await;
        assert_eq!(client.commit_all().await.unwrap(), 3);
        task.await.unwrap();

        let (_directory, client, task) =
            client_with_responses(vec![IpcResponse::JobCount { count: 2 }]).await;
        assert_eq!(
            client.commit_many(vec![id, Uuid::new_v4()]).await.unwrap(),
            2
        );
        task.await.unwrap();

        let (_directory, client, task) =
            client_with_responses(vec![IpcResponse::JobCount { count: 4 }]).await;
        assert_eq!(client.commit_user("alice".to_owned()).await.unwrap(), 4);
        task.await.unwrap();

        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Ack]).await;
        client.commit(id).await.unwrap();
        task.await.unwrap();

        let (_directory, client, task) =
            client_with_responses(vec![IpcResponse::QueuedJobs { jobs: Vec::new() }]).await;
        assert!(client.move_queued(id, 1).await.unwrap().is_empty());
        task.await.unwrap();
        assert_response_error(
            IpcResponse::StaleQueueMove {
                message: "job disappeared".into(),
            },
            |client| async move { client.move_queued(id, 1).await },
            "job disappeared",
        )
        .await;
        assert_response_error(
            IpcResponse::Error {
                message: "move failed".into(),
            },
            |client| async move { client.move_queued(id, 1).await },
            "move failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(sample_status()),
            |client| async move { client.move_queued(id, 1).await },
            "invalid move queued",
        )
        .await;
    }

    #[tokio::test]
    async fn stop_and_follow_logs_handle_success_errors_and_closed_streams() {
        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Ack]).await;
        client.stop().await.unwrap();
        task.await.unwrap();

        assert_response_error(
            IpcResponse::Status(sample_status()),
            |client| async move { client.stop().await },
            "invalid stop",
        )
        .await;
        assert_response_error(
            IpcResponse::Error {
                message: "stop failed".into(),
            },
            |client| async move { client.stop().await },
            "stop failed",
        )
        .await;

        let (_directory, client, task) = client_with_responses(vec![
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogChunk {
                stream: LogStream::Stderr,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ])
        .await;
        client.follow_logs(Uuid::nil()).await.unwrap();
        task.await.unwrap();

        assert_response_error(
            IpcResponse::Error {
                message: "log follow failed".into(),
            },
            |client| async move { client.follow_logs(Uuid::nil()).await },
            "log follow failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.follow_logs(Uuid::nil()).await },
            "invalid log response",
        )
        .await;

        let (_directory, client, task) = client_with_responses(Vec::new()).await;
        let error = client.follow_logs(Uuid::nil()).await.unwrap_err();
        assert!(error.to_string().contains("closed the log stream"));
        task.await.unwrap();
    }
}

#[cfg(all(test, windows))]
mod windows_client_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::future::Future;
    use std::path::Path;
    use tokio::net::windows::named_pipe::ServerOptions;
    use tokio::task::JoinHandle;
    use tokio_util::codec::{Framed, LengthDelimitedCodec};

    fn paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    async fn client_with_response(
        response: IpcResponse,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let server = ServerOptions::new().create(paths.ipc_endpoint()).unwrap();
        let task = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut framed = Framed::new(server, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            framed
                .send(encode_response(&response).unwrap().into())
                .await
                .unwrap();
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn assert_response_error<T, F, Fut>(response: IpcResponse, operation: F, expected: &str)
    where
        F: FnOnce(ServiceClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (_directory, client, task) = client_with_response(response).await;
        let error = operation(client)
            .await
            .err()
            .expect("operation unexpectedly succeeded");
        assert!(error.to_string().contains(expected), "{error:#}");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn commit_batch_clients_handle_success_errors_and_invalid_responses() {
        let id = Uuid::nil();

        let (_directory, client, task) =
            client_with_response(IpcResponse::JobCount { count: 2 }).await;
        assert_eq!(client.commit_many(vec![id]).await.unwrap(), 2);
        task.await.unwrap();

        let (_directory, client, task) =
            client_with_response(IpcResponse::JobCount { count: 4 }).await;
        assert_eq!(client.commit_user("alice".to_owned()).await.unwrap(), 4);
        task.await.unwrap();

        assert_response_error(
            IpcResponse::Error {
                message: "commit many failed".into(),
            },
            |client| async move { client.commit_many(vec![id]).await },
            "commit many failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_many(vec![id]).await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            IpcResponse::Error {
                message: "commit user failed".into(),
            },
            |client| async move { client.commit_user("alice".to_owned()).await },
            "commit user failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_user("alice".to_owned()).await },
            "invalid commit",
        )
        .await;
    }
}
