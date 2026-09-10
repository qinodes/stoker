//! Configuration compatibility façade.
//!
//! Paths remain value-oriented while workspace, repository, snapshot and
//! timezone side effects are implemented by their owning modules.

mod model;
mod paths;
mod repository;
mod snapshots;
mod timezone;
mod workspace;

pub use model::{
    ConfigSnapshot, ConfigSnapshotEntry, ConfigSnapshotFile, ConfigSnapshotReason,
    ResolvedTimezone, StokerConfig, TimezoneSource,
};
pub use paths::StokerPaths;
pub(crate) use paths::normalize_path;
pub use timezone::{resolve_timezone, system_timezone_name};

#[cfg(test)]
mod tests;
