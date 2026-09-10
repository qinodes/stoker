use std::fmt;
use std::path::PathBuf;

use uuid::Uuid;

use crate::domain::{DomainError, JobState};

pub type ApplicationResult<T> = Result<T, ApplicationError>;

/// Stable categories consumed by CLI, HTTP and IPC error mappers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    Unavailable,
    Timeout,
    InvalidDependencyData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependency {
    JobRepository,
    Scheduler,
    Configuration,
    JobArtifacts,
}

impl fmt::Display for Dependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::JobRepository => "job repository",
            Self::Scheduler => "scheduler",
            Self::Configuration => "configuration repository",
            Self::JobArtifacts => "job artifacts",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    CreateJob,
    Status,
    Commit,
    Cancel,
    FollowLogs,
    LockQueue,
    UnlockQueue,
    MoveQueue,
    ReadConfiguration,
    WriteConfiguration,
    CreateSnapshot,
    RestoreSnapshot,
    ReadLogs,
    CleanJobs,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreateJob => "create job",
            Self::Status => "read scheduler status",
            Self::Commit => "commit jobs",
            Self::Cancel => "cancel job",
            Self::FollowLogs => "follow logs",
            Self::LockQueue => "lock queue",
            Self::UnlockQueue => "unlock queue",
            Self::MoveQueue => "move queued job",
            Self::ReadConfiguration => "read configuration",
            Self::WriteConfiguration => "write configuration",
            Self::CreateSnapshot => "create configuration snapshot",
            Self::RestoreSnapshot => "restore configuration snapshot",
            Self::ReadLogs => "read logs",
            Self::CleanJobs => "clean jobs",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Conflict {
    #[error("cannot {operation} job {id} while it is {state}")]
    InvalidJobState {
        id: Uuid,
        state: JobState,
        operation: &'static str,
    },
    #[error(
        "description for job {id} changed concurrently (expected revision {expected_revision}, current revision {actual_revision})"
    )]
    StaleDescription {
        id: Uuid,
        expected_revision: i64,
        actual_revision: i64,
    },
    #[error("queue is locked")]
    QueueLocked,
    #[error("queue is unlocked")]
    QueueUnlocked,
    #[error("queue changed while it was being edited")]
    StaleQueue,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplicationError {
    #[error(transparent)]
    InvalidInput(#[from] DomainError),
    #[error("invalid command: {message}")]
    InvalidCommand { message: String },
    #[error("invalid configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("invalid working directory {}: {message}", path.display())]
    InvalidWorkingDirectory { path: PathBuf, message: String },
    #[error("job {id} does not exist")]
    JobNotFound { id: Uuid },
    #[error("configuration snapshot {} does not exist", path.display())]
    SnapshotNotFound { path: PathBuf },
    #[error(transparent)]
    Conflict(#[from] Conflict),
    #[error("{dependency} is unavailable: {message}")]
    Unavailable {
        dependency: Dependency,
        message: String,
    },
    #[error("{operation} timed out")]
    Timeout { operation: Operation },
    #[error("{dependency} returned invalid data: {message}")]
    InvalidDependencyData {
        dependency: Dependency,
        message: String,
    },
}

impl ApplicationError {
    pub const fn code(&self) -> ApplicationErrorCode {
        match self {
            Self::InvalidInput(_)
            | Self::InvalidCommand { .. }
            | Self::InvalidConfiguration { .. }
            | Self::InvalidWorkingDirectory { .. } => ApplicationErrorCode::InvalidInput,
            Self::JobNotFound { .. } | Self::SnapshotNotFound { .. } => {
                ApplicationErrorCode::NotFound
            }
            Self::Conflict(_) => ApplicationErrorCode::Conflict,
            Self::Unavailable { .. } => ApplicationErrorCode::Unavailable,
            Self::Timeout { .. } => ApplicationErrorCode::Timeout,
            Self::InvalidDependencyData { .. } => ApplicationErrorCode::InvalidDependencyData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ValidationField;

    #[test]
    fn every_application_error_variant_has_a_stable_code() {
        let id = Uuid::nil();
        let cases = [
            (
                ApplicationError::from(DomainError::EmptyValue {
                    field: ValidationField::JobName,
                }),
                ApplicationErrorCode::InvalidInput,
            ),
            (
                ApplicationError::InvalidCommand {
                    message: "empty".to_owned(),
                },
                ApplicationErrorCode::InvalidInput,
            ),
            (
                ApplicationError::InvalidConfiguration {
                    message: "bad timezone".to_owned(),
                },
                ApplicationErrorCode::InvalidInput,
            ),
            (
                ApplicationError::InvalidWorkingDirectory {
                    path: PathBuf::from("missing"),
                    message: "not found".to_owned(),
                },
                ApplicationErrorCode::InvalidInput,
            ),
            (
                ApplicationError::JobNotFound { id },
                ApplicationErrorCode::NotFound,
            ),
            (
                ApplicationError::SnapshotNotFound {
                    path: PathBuf::from("snapshot.json"),
                },
                ApplicationErrorCode::NotFound,
            ),
            (
                ApplicationError::from(Conflict::QueueLocked),
                ApplicationErrorCode::Conflict,
            ),
            (
                ApplicationError::Unavailable {
                    dependency: Dependency::Scheduler,
                    message: "offline".to_owned(),
                },
                ApplicationErrorCode::Unavailable,
            ),
            (
                ApplicationError::Timeout {
                    operation: Operation::FollowLogs,
                },
                ApplicationErrorCode::Timeout,
            ),
            (
                ApplicationError::InvalidDependencyData {
                    dependency: Dependency::JobRepository,
                    message: "bad state".to_owned(),
                },
                ApplicationErrorCode::InvalidDependencyData,
            ),
        ];

        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error}");
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn every_conflict_variant_keeps_typed_details() {
        let id = Uuid::nil();
        let conflicts = [
            Conflict::InvalidJobState {
                id,
                state: JobState::Running,
                operation: "commit",
            },
            Conflict::StaleDescription {
                id,
                expected_revision: 1,
                actual_revision: 2,
            },
            Conflict::QueueLocked,
            Conflict::QueueUnlocked,
            Conflict::StaleQueue,
        ];
        for conflict in conflicts {
            let error = ApplicationError::from(conflict.clone());
            assert_eq!(error.code(), ApplicationErrorCode::Conflict);
            assert!(matches!(error, ApplicationError::Conflict(value) if value == conflict));
        }
    }

    #[test]
    fn operation_and_dependency_names_cover_all_variants() {
        for operation in [
            Operation::CreateJob,
            Operation::Status,
            Operation::Commit,
            Operation::Cancel,
            Operation::FollowLogs,
            Operation::LockQueue,
            Operation::UnlockQueue,
            Operation::MoveQueue,
            Operation::ReadConfiguration,
            Operation::WriteConfiguration,
            Operation::CreateSnapshot,
            Operation::RestoreSnapshot,
            Operation::ReadLogs,
            Operation::CleanJobs,
        ] {
            assert!(!operation.to_string().is_empty());
        }
        for dependency in [
            Dependency::JobRepository,
            Dependency::Scheduler,
            Dependency::Configuration,
            Dependency::JobArtifacts,
        ] {
            assert!(!dependency.to_string().is_empty());
        }
    }
}
