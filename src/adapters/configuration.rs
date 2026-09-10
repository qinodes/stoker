use std::path::Path;

use crate::application::model::{ApplicationConfig, ConfigSnapshot, SnapshotReason};
use crate::application::ports::{
    ConfigurationReader, ConfigurationRepositoryError, ConfigurationSnapshots, ConfigurationWriter,
};
use crate::config::{ConfigSnapshotEntry, ConfigSnapshotReason, StokerConfig, StokerPaths};

impl ConfigurationReader for StokerPaths {
    fn read_configuration(&self) -> Result<ApplicationConfig, ConfigurationRepositoryError> {
        self.read_config()
            .map(|config| ApplicationConfig {
                timezone: config.timezone,
            })
            .map_err(configuration_error)
    }
}

impl ConfigurationWriter for StokerPaths {
    fn write_configuration(
        &self,
        configuration: &ApplicationConfig,
    ) -> Result<(), ConfigurationRepositoryError> {
        self.write_config(&StokerConfig {
            timezone: configuration.timezone.clone(),
        })
        .map_err(configuration_error)
    }
}

impl ConfigurationSnapshots for StokerPaths {
    fn list_snapshots(&self) -> Result<Vec<ConfigSnapshot>, ConfigurationRepositoryError> {
        self.list_config_snapshots()
            .map(|entries| entries.into_iter().map(map_snapshot_entry).collect())
            .map_err(configuration_error)
    }

    fn create_snapshot(
        &self,
        reason: SnapshotReason,
    ) -> Result<ConfigSnapshot, ConfigurationRepositoryError> {
        let config = self.read_config().map_err(configuration_error)?;
        let path = self
            .create_config_snapshot(&config, map_snapshot_reason(reason))
            .map_err(configuration_error)?;
        self.list_config_snapshots()
            .map_err(configuration_error)?
            .into_iter()
            .find(|entry| match entry {
                ConfigSnapshotEntry::Valid(file) => file.path == path,
                ConfigSnapshotEntry::Invalid { path: invalid, .. } => *invalid == path,
            })
            .map(map_snapshot_entry)
            .ok_or_else(|| ConfigurationRepositoryError::InvalidData {
                message: format!("created snapshot {} was not found", path.display()),
            })
    }

    fn restore_snapshot(
        &self,
        path: &Path,
    ) -> Result<ApplicationConfig, ConfigurationRepositoryError> {
        let entry = self
            .list_config_snapshots()
            .map_err(configuration_error)?
            .into_iter()
            .find(|entry| match entry {
                ConfigSnapshotEntry::Valid(file) => file.path == path,
                ConfigSnapshotEntry::Invalid { path: invalid, .. } => invalid == path,
            })
            .ok_or_else(|| ConfigurationRepositoryError::SnapshotNotFound {
                path: path.to_path_buf(),
            })?;
        let ConfigSnapshotEntry::Valid(file) = entry else {
            return Err(ConfigurationRepositoryError::InvalidData {
                message: "the selected configuration snapshot is invalid".to_owned(),
            });
        };
        self.restore_config_snapshot(&file.snapshot)
            .map_err(configuration_error)?;
        Ok(ApplicationConfig {
            timezone: file.snapshot.config.timezone,
        })
    }
}

fn map_snapshot_entry(entry: ConfigSnapshotEntry) -> ConfigSnapshot {
    match entry {
        ConfigSnapshotEntry::Valid(file) => ConfigSnapshot {
            path: file.path,
            valid: true,
            created_at: Some(file.snapshot.created_at),
            reason: Some(match file.snapshot.reason {
                ConfigSnapshotReason::Initial => SnapshotReason::Initial,
                ConfigSnapshotReason::BeforeConfigUpdate => SnapshotReason::BeforeConfigUpdate,
                ConfigSnapshotReason::BeforeRestore => SnapshotReason::BeforeRestore,
                ConfigSnapshotReason::Manual => SnapshotReason::Manual,
            }),
            timezone: file.snapshot.config.timezone,
            error: None,
        },
        ConfigSnapshotEntry::Invalid { path, error } => ConfigSnapshot {
            path,
            valid: false,
            created_at: None,
            reason: None,
            timezone: None,
            error: Some(error),
        },
    }
}

fn map_snapshot_reason(reason: SnapshotReason) -> ConfigSnapshotReason {
    match reason {
        SnapshotReason::Initial => ConfigSnapshotReason::Initial,
        SnapshotReason::BeforeConfigUpdate => ConfigSnapshotReason::BeforeConfigUpdate,
        SnapshotReason::BeforeRestore => ConfigSnapshotReason::BeforeRestore,
        SnapshotReason::Manual => ConfigSnapshotReason::Manual,
    }
}

fn configuration_error(error: anyhow::Error) -> ConfigurationRepositoryError {
    ConfigurationRepositoryError::Unavailable {
        message: format!("{error:#}"),
    }
}
