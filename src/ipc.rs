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

pub const IPC_VERSION: u16 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcRequest {
    Status,
    Stop,
    Commit { id: Uuid },
    CommitAll,
    Pause,
    Resume,
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
            IpcResponse::Ack | IpcResponse::JobCount { .. } | IpcResponse::QueuedJobs { .. } => {
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
            | IpcResponse::QueuedJobs { .. } => {
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

    pub async fn commit_all(&self) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitAll).await? {
            IpcResponse::JobCount { count } => Ok(count),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid commit response"),
        }
    }

    pub async fn pause(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::Pause).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid pause response"),
        }
    }

    pub async fn resume(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::Resume).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid resume response"),
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
                    if error.kind() == std::io::ErrorKind::NotFound
                        && !service_lock_is_available(paths)
                        && tokio::time::Instant::now() < deadline =>
                {
                    // The service creates a fresh named-pipe instance after
                    // each client disconnects. Retry the brief gap before
                    // reporting that the service is unavailable.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Job, JobState};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn frames_include_protocol_version_and_round_trip() {
        let id = Uuid::nil();
        for request in [
            IpcRequest::Status,
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
