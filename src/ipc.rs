//! Versioned local IPC protocol and client.

use std::io::Write;
use std::time::Duration;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;

use crate::StokerPaths;

pub const IPC_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcRequest {
    Status,
    Stop,
    Commit { id: Uuid },
    Cancel { id: Uuid },
    FollowLogs { id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcResponse {
    Ack,
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
            IpcResponse::Ack => anyhow::bail!("service returned an invalid status response"),
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
                    if !self.endpoint_is_reachable().await {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(())
            }
            IpcResponse::Status(_) => anyhow::bail!("service returned an invalid stop response"),
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

    pub async fn cancel(&self, id: Uuid) -> anyhow::Result<()> {
        match self.request(IpcRequest::Cancel { id }).await? {
            IpcResponse::Ack => Ok(()),
            IpcResponse::Error { message } => anyhow::bail!("{message}"),
            _ => anyhow::bail!("service returned an invalid cancel response"),
        }
    }

    /// Follow a job's output and write stdout/stderr chunks to their matching
    /// local streams. The call returns only after the service sends LogEnd.
    pub async fn follow_logs(&self, id: Uuid) -> anyhow::Result<()> {
        let stream = tokio::time::timeout(self.timeout, connect(&self.paths))
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
        let stream = tokio::time::timeout(self.timeout, connect(&self.paths))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_include_protocol_version_and_round_trip() {
        let encoded = encode_request(&IpcRequest::Status).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value["version"], IPC_VERSION);
        assert_eq!(decode_request(&encoded).unwrap(), IpcRequest::Status);

        let response = IpcResponse::Status(ServiceStatus {
            pid: 42,
            active_job: None,
            queued_jobs: 3,
        });
        let encoded = encode_response(&response).unwrap();
        assert_eq!(decode_response(&encoded).unwrap(), response);
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        let frame = serde_json::json!({
            "version": IPC_VERSION + 1,
            "request": "Status"
        });
        let error = decode_request(&serde_json::to_vec(&frame).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported IPC protocol version")
        );
    }
}
