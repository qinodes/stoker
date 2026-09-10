//! Configuration and snapshot use cases.

use std::path::Path;

use crate::application::error::{ApplicationError, ApplicationResult};
use crate::application::model::{ApplicationConfig, ConfigSnapshot, SnapshotReason};
use crate::application::ports::{ConfigurationReader, ConfigurationSnapshots, ConfigurationWriter};

pub fn configuration(reader: &impl ConfigurationReader) -> ApplicationResult<ApplicationConfig> {
    reader.read_configuration().map_err(Into::into)
}

pub fn set_timezone(
    repository: &(impl ConfigurationReader + ConfigurationWriter),
    value: String,
) -> ApplicationResult<ApplicationConfig> {
    if value.trim().is_empty() {
        return Err(ApplicationError::InvalidConfiguration {
            message: "timezone must not be empty".to_owned(),
        });
    }
    if value.parse::<chrono_tz::Tz>().is_err() {
        return Err(ApplicationError::InvalidConfiguration {
            message: format!(
                "unknown timezone {value:?}; use an IANA timezone such as Asia/Taipei or UTC"
            ),
        });
    }
    let mut configuration = repository.read_configuration()?;
    configuration.timezone = Some(value);
    repository.write_configuration(&configuration)?;
    Ok(configuration)
}

pub fn unset_timezone(
    repository: &(impl ConfigurationReader + ConfigurationWriter),
) -> ApplicationResult<ApplicationConfig> {
    let mut configuration = repository.read_configuration()?;
    configuration.timezone = None;
    repository.write_configuration(&configuration)?;
    Ok(configuration)
}

pub fn list_snapshots(
    repository: &impl ConfigurationSnapshots,
) -> ApplicationResult<Vec<ConfigSnapshot>> {
    repository.list_snapshots().map_err(Into::into)
}

pub fn create_snapshot(
    repository: &impl ConfigurationSnapshots,
    reason: SnapshotReason,
) -> ApplicationResult<ConfigSnapshot> {
    repository.create_snapshot(reason).map_err(Into::into)
}

pub fn restore_snapshot(
    repository: &impl ConfigurationSnapshots,
    path: &Path,
) -> ApplicationResult<ApplicationConfig> {
    repository.restore_snapshot(path).map_err(Into::into)
}
