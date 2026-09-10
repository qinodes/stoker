use std::path::PathBuf;

use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

pub(super) const SNAPSHOT_VERSION: u8 = 1;

fn default_snapshot_version() -> u8 {
    SNAPSHOT_VERSION
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StokerConfig {
    #[serde(default)]
    pub timezone: Option<String>,
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
