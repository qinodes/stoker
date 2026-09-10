//! Runtime and persistence failures mapped to stable IPC errors.

use std::path::PathBuf;

use uuid::Uuid;

use crate::domain::JobState;
use crate::ipc::{IpcError, IpcErrorCode, IpcErrorDetails, JobStateDto, ProtocolVersionMismatch};
use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ServiceFailure {
    #[error("scheduler is shutting down; {operation} cancelled")]
    ShuttingDown { operation: &'static str },
    #[error("Job {id} is still {state}; run `stoker commit {id}` before following its logs.")]
    InvalidLogState { id: Uuid, state: JobState },
    #[error("log file {path} does not exist")]
    LogUnavailable { path: PathBuf },
}

pub(crate) fn protocol_error(error: &anyhow::Error, operation: &'static str) -> IpcError {
    if let Some(version) = error.downcast_ref::<ProtocolVersionMismatch>() {
        return IpcError::new(IpcErrorCode::ProtocolVersion, error.to_string()).with_details(
            IpcErrorDetails::ProtocolVersion {
                expected: version.expected,
                actual: version.actual,
            },
        );
    }
    if let Some(failure) = error.downcast_ref::<ServiceFailure>() {
        return match failure {
            ServiceFailure::ShuttingDown { .. } => {
                IpcError::new(IpcErrorCode::ShuttingDown, error.to_string())
            }
            ServiceFailure::InvalidLogState { id, state } => {
                IpcError::new(IpcErrorCode::InvalidState, error.to_string()).with_details(
                    IpcErrorDetails::Job {
                        id: *id,
                        state: Some((*state).into()),
                        operation: operation.to_owned(),
                    },
                )
            }
            ServiceFailure::LogUnavailable { .. } => {
                IpcError::new(IpcErrorCode::NotFound, error.to_string())
            }
        };
    }
    if let Some(store) = error.downcast_ref::<StoreError>() {
        return map_store_error(store, error, operation);
    }
    if error.downcast_ref::<serde_json::Error>().is_some() {
        return IpcError::new(IpcErrorCode::InvalidRequest, format!("{error:#}"));
    }
    IpcError::new(IpcErrorCode::Internal, format!("{error:#}"))
}

fn map_store_error(store: &StoreError, error: &anyhow::Error, operation: &'static str) -> IpcError {
    let message = format!("{error:#}");
    match store {
        StoreError::NotFound { id } => IpcError::new(
            if operation == "move queue" {
                IpcErrorCode::StaleQueue
            } else {
                IpcErrorCode::NotFound
            },
            message,
        )
        .with_details(IpcErrorDetails::Job {
            id: *id,
            state: None,
            operation: operation.to_owned(),
        }),
        StoreError::InvalidTransition { id, state, action } => IpcError::new(
            if *action == "move" {
                IpcErrorCode::StaleQueue
            } else {
                IpcErrorCode::InvalidState
            },
            message,
        )
        .with_details(IpcErrorDetails::Job {
            id: *id,
            state: Some(JobStateDto::from(*state)),
            operation: operation.to_owned(),
        }),
        StoreError::QueueLocked => IpcError::new(IpcErrorCode::QueueLocked, message),
        StoreError::QueueUnlocked => IpcError::new(IpcErrorCode::QueueUnlocked, message),
        StoreError::InvalidQueueOrder {
            id,
            target_order,
            queued_count,
        } => {
            IpcError::new(IpcErrorCode::StaleQueue, message).with_details(IpcErrorDetails::Queue {
                id: Some(*id),
                target_order: Some(*target_order),
                queued_count: Some(*queued_count),
            })
        }
        StoreError::DescriptionConflict { id, .. } => {
            IpcError::new(IpcErrorCode::InvalidState, message).with_details(IpcErrorDetails::Job {
                id: *id,
                state: None,
                operation: operation.to_owned(),
            })
        }
        StoreError::Database(_)
        | StoreError::Serialization(_)
        | StoreError::Poisoned
        | StoreError::InvalidData(_) => IpcError::new(IpcErrorCode::Internal, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_failures_map_without_reading_messages() {
        let id = Uuid::nil();
        let cases = [
            (
                StoreError::NotFound { id },
                "commit",
                IpcErrorCode::NotFound,
            ),
            (
                StoreError::NotFound { id },
                "move queue",
                IpcErrorCode::StaleQueue,
            ),
            (StoreError::QueueLocked, "commit", IpcErrorCode::QueueLocked),
            (
                StoreError::QueueUnlocked,
                "move queue",
                IpcErrorCode::QueueUnlocked,
            ),
            (
                StoreError::InvalidQueueOrder {
                    id,
                    target_order: 3,
                    queued_count: 1,
                },
                "move queue",
                IpcErrorCode::StaleQueue,
            ),
            (
                StoreError::InvalidTransition {
                    id,
                    state: JobState::Succeeded,
                    action: "cancel",
                },
                "cancel",
                IpcErrorCode::InvalidState,
            ),
            (
                StoreError::InvalidTransition {
                    id,
                    state: JobState::Queued,
                    action: "move",
                },
                "move queue",
                IpcErrorCode::StaleQueue,
            ),
            (
                StoreError::DescriptionConflict {
                    id,
                    expected_revision: 1,
                    actual_revision: 2,
                },
                "description",
                IpcErrorCode::InvalidState,
            ),
            (StoreError::Poisoned, "status", IpcErrorCode::Internal),
            (
                StoreError::InvalidData("arbitrary".to_owned()),
                "status",
                IpcErrorCode::Internal,
            ),
            (
                StoreError::Database(rusqlite::Error::InvalidQuery),
                "status",
                IpcErrorCode::Internal,
            ),
            (
                StoreError::Serialization(
                    serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
                ),
                "status",
                IpcErrorCode::Internal,
            ),
        ];
        for (source, operation, expected) in cases {
            let error = anyhow::Error::new(source);
            assert_eq!(protocol_error(&error, operation).code, expected);
        }
    }

    #[test]
    fn runtime_failures_map_to_codes_and_structured_details() {
        let id = Uuid::nil();
        let version = anyhow::Error::new(ProtocolVersionMismatch {
            expected: 4,
            actual: 3,
        });
        assert!(matches!(
            protocol_error(&version, "decode").details,
            Some(IpcErrorDetails::ProtocolVersion {
                expected: 4,
                actual: 3
            })
        ));

        let shutting_down = anyhow::Error::new(ServiceFailure::ShuttingDown {
            operation: "follow logs",
        });
        assert_eq!(
            protocol_error(&shutting_down, "follow logs").code,
            IpcErrorCode::ShuttingDown
        );

        let invalid_state = anyhow::Error::new(ServiceFailure::InvalidLogState {
            id,
            state: JobState::Draft,
        });
        let mapped = protocol_error(&invalid_state, "follow logs");
        assert_eq!(mapped.code, IpcErrorCode::InvalidState);
        assert!(matches!(
            mapped.details,
            Some(IpcErrorDetails::Job {
                id: found,
                state: Some(JobStateDto::Draft),
                ..
            }) if found == id
        ));

        let unavailable = anyhow::Error::new(ServiceFailure::LogUnavailable {
            path: PathBuf::from("missing.log"),
        });
        assert_eq!(
            protocol_error(&unavailable, "follow logs").code,
            IpcErrorCode::NotFound
        );

        let malformed =
            anyhow::Error::new(serde_json::from_str::<serde_json::Value>("{").unwrap_err());
        assert_eq!(
            protocol_error(&malformed, "decode").code,
            IpcErrorCode::InvalidRequest
        );
        assert_eq!(
            protocol_error(&anyhow::anyhow!("unclassified"), "status").code,
            IpcErrorCode::Internal
        );
    }
}
