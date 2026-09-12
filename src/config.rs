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
    DEFAULT_LOG_DISK_RESERVE_BYTES, DEFAULT_LOG_MAX_BYTES_PER_JOB, DEFAULT_LOG_MAX_BYTES_TOTAL,
    DEFAULT_LOG_RETENTION_JOBS, DEFAULT_LOG_SEGMENT_BYTES, DEFAULT_STARTUP_TIMEOUT_MS,
    DEFAULT_TERMINATION_GRACE_MS, LogPolicy, POLICY_MB_BYTES, ResolvedTimezone, RuntimePolicy,
    StokerConfig, TimezoneSource,
};
pub use paths::StokerPaths;
pub(crate) use paths::normalize_path;
pub use timezone::{resolve_timezone, system_timezone_name};

#[cfg(test)]
mod tests;
