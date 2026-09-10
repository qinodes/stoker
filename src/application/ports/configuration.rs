use std::path::{Path, PathBuf};

use crate::application::error::{ApplicationError, Dependency};
use crate::application::model::{ApplicationConfig, ConfigSnapshot, SnapshotReason};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationRepositoryError {
    #[error("configuration snapshot {} does not exist", path.display())]
    SnapshotNotFound { path: PathBuf },
    #[error("configuration repository is unavailable: {message}")]
    Unavailable { message: String },
    #[error("configuration repository returned invalid data: {message}")]
    InvalidData { message: String },
}

impl From<ConfigurationRepositoryError> for ApplicationError {
    fn from(error: ConfigurationRepositoryError) -> Self {
        match error {
            ConfigurationRepositoryError::SnapshotNotFound { path } => {
                Self::SnapshotNotFound { path }
            }
            ConfigurationRepositoryError::Unavailable { message } => Self::Unavailable {
                dependency: Dependency::Configuration,
                message,
            },
            ConfigurationRepositoryError::InvalidData { message } => Self::InvalidDependencyData {
                dependency: Dependency::Configuration,
                message,
            },
        }
    }
}

pub trait ConfigurationReader: Send + Sync {
    fn read_configuration(&self) -> Result<ApplicationConfig, ConfigurationRepositoryError>;
}

pub trait ConfigurationWriter: Send + Sync {
    fn write_configuration(
        &self,
        configuration: &ApplicationConfig,
    ) -> Result<(), ConfigurationRepositoryError>;
}

pub trait ConfigurationSnapshots: Send + Sync {
    fn list_snapshots(&self) -> Result<Vec<ConfigSnapshot>, ConfigurationRepositoryError>;
    fn create_snapshot(
        &self,
        reason: SnapshotReason,
    ) -> Result<ConfigSnapshot, ConfigurationRepositoryError>;
    fn restore_snapshot(
        &self,
        path: &Path,
    ) -> Result<ApplicationConfig, ConfigurationRepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ApplicationErrorCode;

    #[test]
    fn every_configuration_failure_maps_to_a_typed_application_error() {
        let cases = [
            (
                ConfigurationRepositoryError::SnapshotNotFound {
                    path: PathBuf::from("missing.json"),
                },
                ApplicationErrorCode::NotFound,
            ),
            (
                ConfigurationRepositoryError::Unavailable {
                    message: "read failed".to_owned(),
                },
                ApplicationErrorCode::Unavailable,
            ),
            (
                ConfigurationRepositoryError::InvalidData {
                    message: "bad timezone".to_owned(),
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
}
