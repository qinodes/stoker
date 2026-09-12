use std::path::PathBuf;

use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

pub(super) const SNAPSHOT_VERSION: u8 = 1;

/// Number of bytes represented by one user-facing policy megabyte.
pub const POLICY_MB_BYTES: u64 = 1024 * 1024;

pub const DEFAULT_LOG_MAX_BYTES_PER_JOB: u64 = 64 * 1024 * 1024;
pub const DEFAULT_LOG_SEGMENT_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_LOG_MAX_BYTES_TOTAL: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_LOG_RETENTION_JOBS: u64 = 100;
pub const DEFAULT_LOG_DISK_RESERVE_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_TERMINATION_GRACE_MS: u64 = 500;
pub const DEFAULT_STARTUP_TIMEOUT_MS: u64 = 30_000;

fn default_snapshot_version() -> u8 {
    SNAPSHOT_VERSION
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StokerConfig {
    #[serde(default)]
    pub timezone: Option<String>,
}

/// Runtime log retention and disk-pressure policy. These values are stored in
/// SQLite so updates can share the queue-lock transaction with scheduler
/// state, while the timezone configuration remains in the legacy JSON file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPolicy {
    pub max_bytes_per_job: u64,
    pub segment_bytes: u64,
    pub max_bytes_total: u64,
    pub retention_jobs: u64,
    pub disk_reserve_bytes: u64,
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            max_bytes_per_job: DEFAULT_LOG_MAX_BYTES_PER_JOB,
            segment_bytes: DEFAULT_LOG_SEGMENT_BYTES,
            max_bytes_total: DEFAULT_LOG_MAX_BYTES_TOTAL,
            retention_jobs: DEFAULT_LOG_RETENTION_JOBS,
            disk_reserve_bytes: DEFAULT_LOG_DISK_RESERVE_BYTES,
        }
    }
}

impl LogPolicy {
    pub fn validate(self) -> Result<(), String> {
        if self.max_bytes_per_job == 0 {
            return Err("log max bytes per job must be greater than zero".into());
        }
        if self.segment_bytes == 0 {
            return Err("log segment bytes must be greater than zero".into());
        }
        if self.segment_bytes.saturating_mul(2) > self.max_bytes_per_job {
            return Err(
                "log segment bytes cannot exceed half of the shared per-job log limit".into(),
            );
        }
        if self.max_bytes_total < self.max_bytes_per_job {
            return Err("global log limit cannot be smaller than the per-job log limit".into());
        }
        if self.disk_reserve_bytes == 0 {
            return Err("log disk reserve bytes must be greater than zero".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePolicy {
    pub termination_grace_ms: u64,
    pub max_runtime_ms: Option<u64>,
    pub startup_timeout_ms: u64,
}

impl Default for RuntimePolicy {
    fn default() -> Self {
        Self {
            termination_grace_ms: DEFAULT_TERMINATION_GRACE_MS,
            max_runtime_ms: None,
            startup_timeout_ms: DEFAULT_STARTUP_TIMEOUT_MS,
        }
    }
}

impl RuntimePolicy {
    pub fn validate(self) -> Result<(), String> {
        if self.termination_grace_ms == 0 {
            return Err("termination grace must be greater than zero".into());
        }
        if self.startup_timeout_ms == 0 {
            return Err("startup timeout must be greater than zero".into());
        }
        if self.max_runtime_ms == Some(0) {
            return Err("maximum runtime must be greater than zero when set".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSnapshotReason {
    Initial,
    BeforeConfigUpdate,
    BeforeRestore,
    Manual,
}

impl std::fmt::Display for ConfigSnapshotReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Initial => "initial",
            Self::BeforeConfigUpdate => "before config update",
            Self::BeforeRestore => "before restore",
            Self::Manual => "manual",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    #[serde(default = "default_snapshot_version")]
    pub snapshot_version: u8,
    pub created_at: DateTime<Utc>,
    pub reason: ConfigSnapshotReason,
    pub config: StokerConfig,
}

#[derive(Debug, Clone)]
pub struct ConfigSnapshotFile {
    pub path: PathBuf,
    pub snapshot: ConfigSnapshot,
}

#[derive(Debug, Clone)]
pub enum ConfigSnapshotEntry {
    Valid(ConfigSnapshotFile),
    Invalid { path: PathBuf, error: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimezoneSource {
    Cli,
    Config,
    System,
}

#[derive(Debug, Clone)]
pub struct ResolvedTimezone {
    pub name: String,
    pub source: TimezoneSource,
    pub(super) timezone: Tz,
}

impl ResolvedTimezone {
    pub fn format(&self, value: DateTime<Utc>) -> String {
        value
            .with_timezone(&self.timezone)
            .to_rfc3339_opts(SecondsFormat::Millis, false)
    }
}
