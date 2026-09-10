//! Application scheduler gateway implemented by the typed IPC client.

use async_trait::async_trait;
use futures_util::StreamExt;
use uuid::Uuid;

use crate::ServiceClient;
use crate::application::model::{
    CommitSelection, LogEvent, OutputStream, QueueMove, QueueSnapshot, SchedulerStatus,
};
use crate::application::ports::{
    LogEventStream, SchedulerCancelGateway, SchedulerCommitGateway, SchedulerGatewayError,
    SchedulerLogGateway, SchedulerQueueGateway, SchedulerStatusGateway,
};
use crate::application::{Conflict, Operation};
use crate::domain::{Job, JobState};
use crate::ipc::{
    ClientLogEvent, IpcError, IpcErrorCode, IpcErrorDetails, IpcRequest, IpcResponse, JobStateDto,
    LogStream, ServiceRejected, ServiceTimeout, StaleQueueMoveError, is_service_unavailable,
};

pub(crate) type LocalSchedulerGateway = ServiceClient;

#[async_trait]
impl SchedulerStatusGateway for ServiceClient {
    async fn scheduler_status(&self) -> Result<SchedulerStatus, SchedulerGatewayError> {
        self.status()
            .await
            .map(|status| SchedulerStatus {
                pid: status.pid,
                active_job: status.active_job,
                queued_jobs: status.queued_jobs,
                queue_locked: status.queue_locked,
            })
            .map_err(|error| map_gateway_error(error, Operation::Status, None))
    }
}

#[async_trait]
impl SchedulerCommitGateway for ServiceClient {
    async fn commit(&self, selection: CommitSelection) -> Result<Vec<Job>, SchedulerGatewayError> {
        let (request, fallback) = match selection {
            CommitSelection::Jobs(ids) if ids.len() == 1 => (
                IpcRequest::Commit { id: ids[0] },
                Some(Conflict::InvalidJobState {
                    id: ids[0],
                    state: JobState::Draft,
                    operation: "commit",
                }),
            ),
            CommitSelection::Jobs(ids) => (IpcRequest::CommitMany { ids }, None),
            CommitSelection::All => (IpcRequest::CommitAll, None),
            CommitSelection::User(user) => (IpcRequest::CommitUser { user }, None),
        };
        match self
            .request(request)
            .await
            .map_err(|error| map_gateway_error(error, Operation::Commit, fallback.clone()))?
        {
            IpcResponse::Job { job } => Ok(vec![job.into()]),
            IpcResponse::Jobs { jobs } => Ok(jobs.into_iter().map(Into::into).collect()),
            IpcResponse::Error(error) => Err(map_rejection(error, Operation::Commit, fallback)),
            response => Err(invalid_gateway_response("commit", response)),
        }
    }
}

#[async_trait]
impl SchedulerCancelGateway for ServiceClient {
    async fn cancel(&self, id: Uuid) -> Result<Job, SchedulerGatewayError> {
        match self
            .request(IpcRequest::Cancel { id })
            .await
            .map_err(|error| {
                map_gateway_error(
                    error,
                    Operation::Cancel,
                    Some(Conflict::InvalidJobState {
                        id,
                        state: JobState::Running,
                        operation: "cancel",
                    }),
                )
            })? {
            IpcResponse::Job { job } => Ok(job.into()),
            IpcResponse::Error(error) => Err(map_rejection(
                error,
                Operation::Cancel,
                Some(Conflict::InvalidJobState {
                    id,
                    state: JobState::Running,
                    operation: "cancel",
                }),
            )),
            response => Err(invalid_gateway_response("cancel", response)),
        }
    }
}

#[async_trait]
impl SchedulerQueueGateway for ServiceClient {
    async fn set_queue_locked(&self, locked: bool) -> Result<QueueSnapshot, SchedulerGatewayError> {
        let (request, operation) = if locked {
            (IpcRequest::LockQueue, Operation::LockQueue)
        } else {
            (IpcRequest::UnlockQueue, Operation::UnlockQueue)
        };
        match self
            .request(request)
            .await
            .map_err(|error| map_gateway_error(error, operation, None))?
        {
            IpcResponse::Queue { jobs, locked } => Ok(QueueSnapshot {
                jobs: jobs.into_iter().map(Into::into).collect(),
                locked,
            }),
            IpcResponse::Error(error) => Err(map_rejection(error, operation, None)),
            response => Err(invalid_gateway_response("queue lock", response)),
        }
    }

    async fn move_queued(
        &self,
        movement: QueueMove,
    ) -> Result<QueueSnapshot, SchedulerGatewayError> {
        match self
            .request(IpcRequest::MoveQueued {
                id: movement.id,
                target_order: movement.target_order,
            })
            .await
            .map_err(|error| {
                map_gateway_error(error, Operation::MoveQueue, Some(Conflict::StaleQueue))
            })? {
            IpcResponse::Queue { jobs, locked } => Ok(QueueSnapshot {
                jobs: jobs.into_iter().map(Into::into).collect(),
                locked,
            }),
            IpcResponse::Error(error) => Err(map_rejection(
                error,
                Operation::MoveQueue,
                Some(Conflict::StaleQueue),
            )),
            response => Err(invalid_gateway_response("move queue", response)),
        }
    }
}

#[async_trait]
impl SchedulerLogGateway for ServiceClient {
    async fn follow_logs(&self, id: Uuid) -> Result<LogEventStream, SchedulerGatewayError> {
        let stream = ServiceClient::follow_log_stream(self, id)
            .await
            .map_err(|error| map_gateway_error(error, Operation::FollowLogs, None))?;
        Ok(Box::pin(stream.map(|event| match event {
            Ok(ClientLogEvent::Chunk { stream, bytes }) => Ok(LogEvent::Chunk {
                stream: match stream {
                    LogStream::Stdout => OutputStream::Stdout,
                    LogStream::Stderr => OutputStream::Stderr,
                },
                bytes,
            }),
            Ok(ClientLogEvent::End) => Ok(LogEvent::End),
            Err(error) => Err(map_gateway_error(error, Operation::FollowLogs, None)),
        })))
    }
}

fn map_rejection(
    error: IpcError,
    operation: Operation,
    fallback: Option<Conflict>,
) -> SchedulerGatewayError {
    match error.code {
        IpcErrorCode::QueueLocked => SchedulerGatewayError::Rejected(Conflict::QueueLocked),
        IpcErrorCode::QueueUnlocked => SchedulerGatewayError::Rejected(Conflict::QueueUnlocked),
        IpcErrorCode::StaleQueue => SchedulerGatewayError::Rejected(Conflict::StaleQueue),
        IpcErrorCode::InvalidState => {
            let conflict = job_state_conflict(&error).or(fallback);
            conflict.map_or_else(
                || SchedulerGatewayError::InvalidData {
                    message: error.message,
                },
                SchedulerGatewayError::Rejected,
            )
        }
        IpcErrorCode::NotFound => job_error_id(&error).map_or_else(
            || SchedulerGatewayError::InvalidData {
                message: error.message,
            },
            |id| SchedulerGatewayError::JobNotFound { id },
        ),
        IpcErrorCode::Timeout => SchedulerGatewayError::Timeout { operation },
        IpcErrorCode::Unavailable => SchedulerGatewayError::Unavailable {
            message: error.message,
        },
        IpcErrorCode::InvalidRequest
        | IpcErrorCode::ProtocolVersion
        | IpcErrorCode::ShuttingDown
        | IpcErrorCode::Internal => SchedulerGatewayError::InvalidData {
            message: error.message,
        },
    }
}

fn job_state_conflict(error: &IpcError) -> Option<Conflict> {
    let IpcErrorDetails::Job {
        id,
        state: Some(state),
        operation,
    } = error.details.as_ref()?
    else {
        return None;
    };
    Some(Conflict::InvalidJobState {
        id: *id,
        state: job_state(*state),
        operation: if operation == "cancel" {
            "cancel"
        } else {
            "commit"
        },
    })
}

fn job_error_id(error: &IpcError) -> Option<Uuid> {
    let IpcErrorDetails::Job { id, .. } = error.details.as_ref()? else {
        return None;
    };
    Some(*id)
}

fn job_state(state: JobStateDto) -> JobState {
    state.into()
}

fn map_gateway_error(
    error: anyhow::Error,
    operation: Operation,
    fallback: Option<Conflict>,
) -> SchedulerGatewayError {
    if is_service_unavailable(&error) {
        return SchedulerGatewayError::Unavailable {
            message: error.to_string(),
        };
    }
    if error.downcast_ref::<StaleQueueMoveError>().is_some() {
        return SchedulerGatewayError::Rejected(Conflict::StaleQueue);
    }
    if error.downcast_ref::<ServiceTimeout>().is_some() {
        return SchedulerGatewayError::Timeout { operation };
    }
    if let Some(rejected) = error.downcast_ref::<ServiceRejected>() {
        return map_rejection(rejected.error.clone(), operation, fallback);
    }
    SchedulerGatewayError::InvalidData {
        message: error.to_string(),
    }
}

fn invalid_gateway_response(
    operation: &'static str,
    response: IpcResponse,
) -> SchedulerGatewayError {
    SchedulerGatewayError::InvalidData {
        message: format!("invalid {operation} response: {response:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn paths(root: &Path) -> crate::StokerPaths {
        crate::StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    #[test]
    fn typed_protocol_errors_map_without_message_parsing() {
        let id = Uuid::nil();
        let error = IpcError::new(IpcErrorCode::InvalidState, "arbitrary wording").with_details(
            IpcErrorDetails::Job {
                id,
                state: Some(JobStateDto::Succeeded),
                operation: "cancel".to_owned(),
            },
        );
        assert!(matches!(
            map_rejection(error, Operation::Cancel, None),
            SchedulerGatewayError::Rejected(Conflict::InvalidJobState {
                id: found,
                state: JobState::Succeeded,
                operation: "cancel",
            }) if found == id
        ));
        assert!(matches!(
            map_rejection(
                IpcError::new(IpcErrorCode::QueueLocked, "different wording"),
                Operation::Commit,
                None,
            ),
            SchedulerGatewayError::Rejected(Conflict::QueueLocked)
        ));
        assert!(matches!(
            map_rejection(
                IpcError::new(IpcErrorCode::NotFound, "wording is irrelevant").with_details(
                    IpcErrorDetails::Job {
                        id,
                        state: None,
                        operation: "cancel".to_owned(),
                    },
                ),
                Operation::Cancel,
                None,
            ),
            SchedulerGatewayError::JobNotFound { id: found } if found == id
        ));

        for (code, expected) in [
            (IpcErrorCode::QueueUnlocked, "queue_unlocked"),
            (IpcErrorCode::StaleQueue, "stale_queue"),
            (IpcErrorCode::Timeout, "timeout"),
            (IpcErrorCode::Unavailable, "unavailable"),
            (IpcErrorCode::InvalidRequest, "invalid_data"),
            (IpcErrorCode::ProtocolVersion, "invalid_data"),
            (IpcErrorCode::ShuttingDown, "invalid_data"),
            (IpcErrorCode::Internal, "invalid_data"),
        ] {
            let mapped = map_rejection(
                IpcError::new(code, "the wording can change"),
                Operation::MoveQueue,
                None,
            );
            let actual = match mapped {
                SchedulerGatewayError::Rejected(Conflict::QueueUnlocked) => "queue_unlocked",
                SchedulerGatewayError::Rejected(Conflict::StaleQueue) => "stale_queue",
                SchedulerGatewayError::Timeout { .. } => "timeout",
                SchedulerGatewayError::Unavailable { .. } => "unavailable",
                SchedulerGatewayError::InvalidData { .. } => "invalid_data",
                other => panic!("unexpected mapping for {code:?}: {other:?}"),
            };
            assert_eq!(actual, expected);
        }

        assert!(matches!(
            map_rejection(
                IpcError::new(IpcErrorCode::InvalidState, "no details"),
                Operation::Commit,
                None,
            ),
            SchedulerGatewayError::InvalidData { .. }
        ));
        assert!(matches!(
            map_rejection(
                IpcError::new(IpcErrorCode::NotFound, "no details"),
                Operation::Commit,
                None,
            ),
            SchedulerGatewayError::InvalidData { .. }
        ));
    }

    #[test]
    fn transport_failures_map_by_type_instead_of_display_text() {
        let timeout = anyhow::Error::new(ServiceTimeout {
            operation: "arbitrary",
        });
        assert!(matches!(
            map_gateway_error(timeout, Operation::Status, None),
            SchedulerGatewayError::Timeout {
                operation: Operation::Status
            }
        ));

        let unavailable = anyhow::Error::new(crate::ipc::ServiceUnavailable(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "arbitrary",
        )));
        assert!(matches!(
            map_gateway_error(unavailable, Operation::Status, None),
            SchedulerGatewayError::Unavailable { .. }
        ));

        let stale = anyhow::Error::new(StaleQueueMoveError::new("arbitrary".to_owned()));
        assert!(matches!(
            map_gateway_error(stale, Operation::MoveQueue, None),
            SchedulerGatewayError::Rejected(Conflict::StaleQueue)
        ));

        let rejected = anyhow::Error::new(ServiceRejected::new(IpcError::new(
            IpcErrorCode::QueueLocked,
            "arbitrary",
        )));
        assert!(matches!(
            map_gateway_error(rejected, Operation::Commit, None),
            SchedulerGatewayError::Rejected(Conflict::QueueLocked)
        ));

        assert!(matches!(
            map_gateway_error(anyhow::anyhow!("arbitrary"), Operation::Status, None),
            SchedulerGatewayError::InvalidData { .. }
        ));
    }

    #[tokio::test]
    async fn missing_service_is_typed_consistently_for_every_scheduler_capability() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let gateway = ServiceClient::new(paths);

        assert!(matches!(
            gateway.scheduler_status().await,
            Err(SchedulerGatewayError::Unavailable { .. })
        ));
        for selection in [
            CommitSelection::Jobs(vec![Uuid::nil()]),
            CommitSelection::Jobs(Vec::new()),
            CommitSelection::All,
            CommitSelection::User("alice".to_owned()),
        ] {
            assert!(matches!(
                SchedulerCommitGateway::commit(&gateway, selection).await,
                Err(SchedulerGatewayError::Unavailable { .. })
            ));
        }
        assert!(matches!(
            SchedulerCancelGateway::cancel(&gateway, Uuid::nil()).await,
            Err(SchedulerGatewayError::Unavailable { .. })
        ));
        assert!(matches!(
            gateway.set_queue_locked(true).await,
            Err(SchedulerGatewayError::Unavailable { .. })
        ));
        assert!(matches!(
            SchedulerQueueGateway::move_queued(
                &gateway,
                QueueMove {
                    id: Uuid::nil(),
                    target_order: 1,
                },
            )
            .await,
            Err(SchedulerGatewayError::Unavailable { .. })
        ));
        assert!(matches!(
            SchedulerLogGateway::follow_logs(&gateway, Uuid::nil()).await,
            Err(SchedulerGatewayError::Unavailable { .. })
        ));
    }
}
