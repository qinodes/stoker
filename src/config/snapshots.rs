use std::cmp::Ordering;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use chrono::Utc;
use uuid::Uuid;

use super::model::{
    ConfigSnapshot, ConfigSnapshotEntry, ConfigSnapshotFile, ConfigSnapshotReason,
    SNAPSHOT_VERSION, StokerConfig,
};
use super::paths::StokerPaths;
use super::{repository, timezone};

impl StokerPaths {
    pub fn create_config_snapshot(
        &self,
        config: &StokerConfig,
        reason: ConfigSnapshotReason,
    ) -> anyhow::Result<PathBuf> {
        create(self, config, reason)
    }

    pub fn list_config_snapshots(&self) -> anyhow::Result<Vec<ConfigSnapshotEntry>> {
        list(self)
    }

    pub fn restore_config_snapshot(&self, snapshot: &ConfigSnapshot) -> anyhow::Result<bool> {
        restore(self, snapshot)
    }
}

pub(super) fn create(
    paths: &StokerPaths,
    config: &StokerConfig,
    reason: ConfigSnapshotReason,
) -> anyhow::Result<PathBuf> {
    timezone::validate_config(config)?;
    let directory = paths.snapshot_dir();
    fs::create_dir_all(&directory)
        .with_context(|| format!("create Stoker snapshot directory {}", directory.display()))?;

    let created_at = Utc::now();
    let timestamp = format!(
        "{}{:09}Z",
        created_at.format("%Y%m%dT%H%M%S"),
        created_at.timestamp_subsec_nanos()
    );
    let path = directory.join(format!("config-{timestamp}-{}.json", Uuid::new_v4()));
    let snapshot = ConfigSnapshot {
        snapshot_version: SNAPSHOT_VERSION,
        created_at,
        reason,
        config: config.clone(),
    };
    let contents = serde_json::to_string_pretty(&snapshot)? + "\n";
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("create Stoker config snapshot {}", path.display()))?;
    if let Err(error) = file.write_all(contents.as_bytes()) {
        let _ = fs::remove_file(&path);
        return Err(error)
            .with_context(|| format!("write Stoker config snapshot {}", path.display()));
    }
    if let Err(error) = file.sync_all() {
        let _ = fs::remove_file(&path);
        return Err(error)
            .with_context(|| format!("sync Stoker config snapshot {}", path.display()));
    }
    Ok(path)
}

fn list(paths: &StokerPaths) -> anyhow::Result<Vec<ConfigSnapshotEntry>> {
    let directory = paths.snapshot_dir();
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("read Stoker snapshot directory {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        match read_snapshot(&path) {
            Ok(snapshot) => entries.push(ConfigSnapshotEntry::Valid(ConfigSnapshotFile {
                path,
                snapshot,
            })),
            Err(error) => entries.push(ConfigSnapshotEntry::Invalid {
                path,
                error: format!("{error:#}"),
            }),
        }
    }

    entries.sort_by(|left, right| match (left, right) {
        (ConfigSnapshotEntry::Valid(left), ConfigSnapshotEntry::Valid(right)) => {
            right.snapshot.created_at.cmp(&left.snapshot.created_at)
        }
        (ConfigSnapshotEntry::Valid(_), ConfigSnapshotEntry::Invalid { .. }) => Ordering::Less,
        (ConfigSnapshotEntry::Invalid { .. }, ConfigSnapshotEntry::Valid(_)) => Ordering::Greater,
        (
            ConfigSnapshotEntry::Invalid { path: left, .. },
            ConfigSnapshotEntry::Invalid { path: right, .. },
        ) => right.cmp(left),
    });
    Ok(entries)
}

fn read_snapshot(path: &std::path::Path) -> anyhow::Result<ConfigSnapshot> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("read snapshot {}", path.display()))?;
    let snapshot: ConfigSnapshot = serde_json::from_str(&contents)
        .with_context(|| format!("parse snapshot {}", path.display()))?;
    if snapshot.snapshot_version != SNAPSHOT_VERSION {
        anyhow::bail!("unsupported snapshot version {}", snapshot.snapshot_version);
    }
    timezone::validate_config(&snapshot.config)?;
    Ok(snapshot)
}

fn restore(paths: &StokerPaths, snapshot: &ConfigSnapshot) -> anyhow::Result<bool> {
    timezone::validate_config(&snapshot.config)?;
    let current = repository::read(paths)?;
    if current == snapshot.config {
        return Ok(false);
    }
    create(paths, &current, ConfigSnapshotReason::BeforeRestore)?;
    repository::write_contents(paths, &snapshot.config)?;
    Ok(true)
}
