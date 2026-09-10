//! Typed IPC client and event-based log streaming.

use std::pin::Pin;
use std::time::Duration;

use futures_util::{SinkExt, Stream, StreamExt};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;

use super::error::{
    InvalidServiceResponse, ServiceRejected, ServiceTimeout, ServiceUnavailable,
    StaleQueueMoveError,
};
use super::framing::{decode_response, encode_request};
use super::protocol::{IpcErrorCode, IpcRequest, IpcResponse, LogStream, ServiceStatus};
use super::transport::{connect, connect_with_retry, service_lock_is_available};
use crate::{Job, StokerPaths};

#[derive(Debug, Clone)]
pub struct ServiceClient {
    paths: StokerPaths,
    timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientLogEvent {
    Chunk { stream: LogStream, bytes: Vec<u8> },
    End,
}

pub type ClientLogStream =
    Pin<Box<dyn Stream<Item = anyhow::Result<ClientLogEvent>> + Send + 'static>>;

impl ServiceClient {
    pub fn new(paths: StokerPaths) -> Self {
        Self {
            paths,
            timeout: Duration::from_secs(2),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_timeout(paths: StokerPaths, timeout: Duration) -> Self {
        Self { paths, timeout }
    }

    pub async fn status(&self) -> anyhow::Result<ServiceStatus> {
        match self.request(IpcRequest::Status).await? {
            IpcResponse::Status(status) => Ok(status),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("status", &response)),
        }
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::Stop).await? {
            IpcResponse::Ack => {
                let deadline = tokio::time::Instant::now() + self.timeout;
                loop {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(ServiceTimeout { operation: "stop" }.into());
                    }
                    if !self.endpoint_is_reachable().await && self.service_lock_is_available() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(())
            }
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("stop", &response)),
        }
    }

    pub async fn commit(&self, id: Uuid) -> anyhow::Result<()> {
        match self.request(IpcRequest::Commit { id }).await? {
            IpcResponse::Job { .. } => Ok(()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("commit", &response)),
        }
    }

    pub async fn commit_many(&self, ids: Vec<Uuid>) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitMany { ids }).await? {
            IpcResponse::Jobs { jobs } => Ok(jobs.len()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("commit many", &response)),
        }
    }

    pub async fn commit_all(&self) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitAll).await? {
            IpcResponse::Jobs { jobs } => Ok(jobs.len()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("commit all", &response)),
        }
    }

    pub async fn commit_user(&self, user: String) -> anyhow::Result<usize> {
        match self.request(IpcRequest::CommitUser { user }).await? {
            IpcResponse::Jobs { jobs } => Ok(jobs.len()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("commit user", &response)),
        }
    }

    pub async fn cancel(&self, id: Uuid) -> anyhow::Result<()> {
        match self.request(IpcRequest::Cancel { id }).await? {
            IpcResponse::Job { .. } => Ok(()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("cancel", &response)),
        }
    }

    pub async fn lock_queue(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::LockQueue).await? {
            IpcResponse::Queue { .. } => Ok(()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("lock queue", &response)),
        }
    }

    pub async fn unlock_queue(&self) -> anyhow::Result<()> {
        match self.request(IpcRequest::UnlockQueue).await? {
            IpcResponse::Queue { .. } => Ok(()),
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("unlock queue", &response)),
        }
    }

    pub async fn move_queued(&self, id: Uuid, target_order: usize) -> anyhow::Result<Vec<Job>> {
        match self
            .request(IpcRequest::MoveQueued { id, target_order })
            .await?
        {
            IpcResponse::Queue { jobs, .. } => Ok(jobs.into_iter().map(Into::into).collect()),
            IpcResponse::Error(error) if error.code == IpcErrorCode::StaleQueue => {
                Err(StaleQueueMoveError::new(error.message).into())
            }
            IpcResponse::Error(error) => Err(ServiceRejected::new(error).into()),
            response => Err(invalid_response("move queued", &response)),
        }
    }

    /// Open an event stream for a job's stdout and stderr output.
    pub async fn follow_log_stream(&self, id: Uuid) -> anyhow::Result<ClientLogStream> {
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
            .map_err(|_| ServiceTimeout {
                operation: "send follow logs",
            })??;
        Ok(Box::pin(futures_util::stream::unfold(
            (framed, false),
            |(mut framed, done)| async move {
                if done {
                    return None;
                }
                let item = match framed.next().await {
                    Some(Ok(frame)) => match decode_response(&frame) {
                        Ok(IpcResponse::LogChunk { stream, bytes }) => {
                            Ok(ClientLogEvent::Chunk { stream, bytes })
                        }
                        Ok(IpcResponse::LogEnd) => Ok(ClientLogEvent::End),
                        Ok(IpcResponse::Error(error)) => Err(ServiceRejected::new(error).into()),
                        Ok(response) => Err(invalid_response("log", &response)),
                        Err(error) => Err(error),
                    },
                    Some(Err(error)) => Err(error.into()),
                    None => Err(anyhow::anyhow!("scheduler closed the log stream")),
                };
                let finished = matches!(item, Ok(ClientLogEvent::End)) || item.is_err();
                Some((item, (framed, finished)))
            },
        )))
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
            .map_err(|_| ServiceTimeout { operation: "send" })?
            .map_err(|error| anyhow::Error::new(ServiceUnavailable(error)))?;
        let frame = tokio::time::timeout(self.timeout, framed.next())
            .await
            .map_err(|_| ServiceTimeout {
                operation: "wait for scheduler response",
            })?
            .ok_or_else(|| anyhow::anyhow!("scheduler closed the IPC connection"))??;
        decode_response(&frame)
    }

    async fn endpoint_is_reachable(&self) -> bool {
        match tokio::time::timeout(Duration::from_millis(100), connect(&self.paths)).await {
            Ok(Ok(_)) => true,
            Ok(Err(_)) => false,
            Err(_) => true,
        }
    }

    fn service_lock_is_available(&self) -> bool {
        service_lock_is_available(&self.paths)
    }
}

fn invalid_response(operation: &'static str, response: &IpcResponse) -> anyhow::Error {
    let response = match response {
        IpcResponse::Ack => "ack",
        IpcResponse::Job { .. } => "job",
        IpcResponse::Jobs { .. } => "jobs",
        IpcResponse::Queue { .. } => "queue",
        IpcResponse::Status(_) => "status",
        IpcResponse::LogChunk { .. } => "log chunk",
        IpcResponse::LogEnd => "log end",
        IpcResponse::Error(_) => "error",
    };
    InvalidServiceResponse {
        operation,
        response,
    }
    .into()
}
