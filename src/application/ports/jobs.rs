use uuid::Uuid;

use crate::application::error::{ApplicationError, Conflict, Dependency};
use crate::application::model::{
    CommitSelection, DescriptionUpdate, JobFilter, LogContent, OutputStream, PreparedJobInput,
    QueueMove, QueueSnapshot,
};
use crate::domain::Job;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobRepositoryError {
    #[error("job {id} does not exist")]
    NotFound { id: Uuid },
    #[error(transparent)]
    Conflict(Conflict),
    #[error("job repository is unavailable: {message}")]
    Unavailable { message: String },
    #[error("job repository returned invalid data: {message}")]
    InvalidData { message: String },
}

impl From<JobRepositoryError> for ApplicationError {
    fn from(error: JobRepositoryError) -> Self {
        match error {
            JobRepositoryError::NotFound { id } => Self::JobNotFound { id },
            JobRepositoryError::Conflict(conflict) => Self::Conflict(conflict),
            JobRepositoryError::Unavailable { message } => Self::Unavailable {
                dependency: Dependency::JobRepository,
                message,
            },
            JobRepositoryError::InvalidData { message } => Self::InvalidDependencyData {
                dependency: Dependency::JobRepository,
                message,
            },
        }
    }
}

pub trait JobQueries: Send + Sync {
    fn get_job(&self, id: Uuid) -> Result<Job, JobRepositoryError>;
    fn list_jobs(&self, filter: &JobFilter) -> Result<Vec<Job>, JobRepositoryError>;
}

pub trait JobCreator: Send + Sync {
    fn create_job(&self, input: PreparedJobInput) -> Result<Job, JobRepositoryError>;
}

pub trait WorkingDirectoryResolver: Send + Sync {
    fn resolve_working_directory(
        &self,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobArtifactsError {
    #[error("job artifacts are unavailable: {message}")]
    Unavailable { message: String },
    #[error("job artifacts returned invalid data: {message}")]
    InvalidData { message: String },
}

impl From<JobArtifactsError> for ApplicationError {
    fn from(error: JobArtifactsError) -> Self {
        match error {
            JobArtifactsError::Unavailable { message } => Self::Unavailable {
                dependency: Dependency::JobArtifacts,
                message,
            },
            JobArtifactsError::InvalidData { message } => Self::InvalidDependencyData {
                dependency: Dependency::JobArtifacts,
                message,
            },
        }
    }
}

pub trait JobArtifacts: Send + Sync {
    fn remove_job_artifacts(&self, id: Uuid) -> Result<(), JobArtifactsError>;
    fn read_log(
        &self,
        id: Uuid,
        stream: OutputStream,
        max_bytes: Option<usize>,
    ) -> Result<LogContent, JobArtifactsError>;
}

pub trait FlowAttemptArtifacts: Send + Sync {
    fn read_flow_attempt_log(
        &self,
        run_id: Uuid,
        task_id: &str,
        attempt: u32,
        stream: OutputStream,
        max_bytes: Option<usize>,
    ) -> Result<LogContent, JobArtifactsError>;
}

pub trait DescriptionUpdater: Send + Sync {
    fn update_description(&self, input: DescriptionUpdate) -> Result<Job, JobRepositoryError>;
}

pub trait JobCleaner: Send + Sync {
    fn clean_terminal_jobs(&self) -> Result<Vec<Job>, JobRepositoryError>;
}

pub trait JobCommitter: Send + Sync {
    fn commit_jobs(&self, selection: &CommitSelection) -> Result<Vec<Job>, JobRepositoryError>;
}

pub trait JobCanceller: Send + Sync {
    fn cancel_not_started(&self, id: Uuid) -> Result<Job, JobRepositoryError>;
}

pub trait QueueRepository: Send + Sync {
    fn queue_snapshot(&self) -> Result<QueueSnapshot, JobRepositoryError>;
    fn set_queue_locked(&self, locked: bool) -> Result<QueueSnapshot, JobRepositoryError>;
    fn move_queued(&self, movement: QueueMove) -> Result<QueueSnapshot, JobRepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{ApplicationErrorCode, Conflict};

    #[test]
    fn every_job_repository_failure_maps_to_a_typed_application_error() {
        let id = Uuid::nil();
        let cases = [
            (
                JobRepositoryError::NotFound { id },
                ApplicationErrorCode::NotFound,
            ),
            (
                JobRepositoryError::Conflict(Conflict::QueueLocked),
                ApplicationErrorCode::Conflict,
            ),
            (
                JobRepositoryError::Unavailable {
                    message: "database busy".to_owned(),
                },
                ApplicationErrorCode::Unavailable,
            ),
            (
                JobRepositoryError::InvalidData {
                    message: "unknown state".to_owned(),
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
    fn every_job_artifact_failure_maps_to_the_artifact_dependency() {
        for (failure, expected) in [
            (
                JobArtifactsError::Unavailable {
                    message: "permission denied".to_owned(),
                },
                ApplicationErrorCode::Unavailable,
            ),
            (
                JobArtifactsError::InvalidData {
                    message: "invalid bytes".to_owned(),
                },
                ApplicationErrorCode::InvalidDependencyData,
            ),
        ] {
            let error = ApplicationError::from(failure);
            assert_eq!(error.code(), expected);
            assert!(matches!(
                error,
                ApplicationError::Unavailable {
                    dependency: Dependency::JobArtifacts,
                    ..
                } | ApplicationError::InvalidDependencyData {
                    dependency: Dependency::JobArtifacts,
                    ..
                }
            ));
        }
    }
}
