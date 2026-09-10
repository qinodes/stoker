use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use uuid::Uuid;

use crate::application::error::{ApplicationError, Conflict, Dependency, Operation};
use crate::application::model::{
    CommitSelection, LogEvent, QueueMove, QueueSnapshot, SchedulerStatus,
};
use crate::domain::Job;

pub type LogEventStream =
    Pin<Box<dyn Stream<Item = Result<LogEvent, SchedulerGatewayError>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchedulerGatewayError {
    #[error("job {id} does not exist")]
    JobNotFound { id: Uuid },
    #[error(transparent)]
    Rejected(Conflict),
    #[error("scheduler is unavailable: {message}")]
    Unavailable { message: String },
    #[error("{operation} timed out")]
    Timeout { operation: Operation },
    #[error("scheduler returned invalid data: {message}")]
    InvalidData { message: String },
}

impl From<SchedulerGatewayError> for ApplicationError {
    fn from(error: SchedulerGatewayError) -> Self {
        match error {
            SchedulerGatewayError::JobNotFound { id } => Self::JobNotFound { id },
            SchedulerGatewayError::Rejected(conflict) => Self::Conflict(conflict),
            SchedulerGatewayError::Unavailable { message } => Self::Unavailable {
                dependency: Dependency::Scheduler,
                message,
            },
            SchedulerGatewayError::Timeout { operation } => Self::Timeout { operation },
            SchedulerGatewayError::InvalidData { message } => Self::InvalidDependencyData {
                dependency: Dependency::Scheduler,
                message,
            },
        }
    }
}

#[async_trait]
pub trait SchedulerStatusGateway: Send + Sync {
    async fn scheduler_status(&self) -> Result<SchedulerStatus, SchedulerGatewayError>;
}

#[async_trait]
pub trait SchedulerCommitGateway: Send + Sync {
    async fn commit(&self, selection: CommitSelection) -> Result<Vec<Job>, SchedulerGatewayError>;
}

#[async_trait]
pub trait SchedulerCancelGateway: Send + Sync {
    async fn cancel(&self, id: Uuid) -> Result<Job, SchedulerGatewayError>;
}

#[async_trait]
pub trait SchedulerQueueGateway: Send + Sync {
    async fn set_queue_locked(&self, locked: bool) -> Result<QueueSnapshot, SchedulerGatewayError>;
    async fn move_queued(
        &self,
        movement: QueueMove,
    ) -> Result<QueueSnapshot, SchedulerGatewayError>;
}

#[async_trait]
pub trait SchedulerLogGateway: Send + Sync {
    async fn follow_logs(&self, id: Uuid) -> Result<LogEventStream, SchedulerGatewayError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{ApplicationErrorCode, LogEvent, OutputStream};
    use futures_util::{stream, task::noop_waker_ref};
    use std::task::{Context, Poll};

    #[test]
    fn every_scheduler_failure_maps_to_a_typed_application_error() {
        let cases = [
            (
                SchedulerGatewayError::JobNotFound { id: Uuid::nil() },
                ApplicationErrorCode::NotFound,
            ),
            (
                SchedulerGatewayError::Rejected(Conflict::StaleQueue),
                ApplicationErrorCode::Conflict,
            ),
            (
                SchedulerGatewayError::Unavailable {
                    message: "offline".to_owned(),
                },
                ApplicationErrorCode::Unavailable,
            ),
            (
                SchedulerGatewayError::Timeout {
                    operation: Operation::Status,
                },
                ApplicationErrorCode::Timeout,
            ),
            (
                SchedulerGatewayError::InvalidData {
                    message: "unknown response".to_owned(),
                },
                ApplicationErrorCode::InvalidDependencyData,
            ),
        ];
        for (failure, expected) in cases {
            let error = ApplicationError::from(failure);
            assert_eq!(error.code(), expected);
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn log_stream_exposes_events_without_owning_console_output() {
        let mut events: LogEventStream = Box::pin(stream::iter([
            Ok(LogEvent::Chunk {
                stream: OutputStream::Stdout,
                bytes: b"hello".to_vec(),
            }),
            Ok(LogEvent::End),
        ]));
        let mut context = Context::from_waker(noop_waker_ref());
        assert_eq!(
            events.as_mut().poll_next(&mut context),
            Poll::Ready(Some(Ok(LogEvent::Chunk {
                stream: OutputStream::Stdout,
                bytes: b"hello".to_vec(),
            })))
        );
        assert_eq!(
            events.as_mut().poll_next(&mut context),
            Poll::Ready(Some(Ok(LogEvent::End)))
        );
        assert_eq!(events.as_mut().poll_next(&mut context), Poll::Ready(None));
    }
}
