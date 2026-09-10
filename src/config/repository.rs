use std::fs;

use anyhow::Context;

use super::model::{ConfigSnapshotReason, StokerConfig};
use super::paths::StokerPaths;
use super::{snapshots, timezone};

impl StokerPaths {
    pub fn read_config(&self) -> anyhow::Result<StokerConfig> {
        read(self)
    }

    pub fn write_config(&self, config: &StokerConfig) -> anyhow::Result<()> {
        write(self, config)
    }
}

pub(super) fn read(paths: &StokerPaths) -> anyhow::Result<StokerConfig> {
    let path = paths.config_path();
    if !path.exists() {
        return Ok(StokerConfig::default());
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read Stoker config {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("parse Stoker config {}", path.display()))
}

pub(super) fn write(paths: &StokerPaths, config: &StokerConfig) -> anyhow::Result<()> {
    timezone::validate_config(config)?;
    let path = paths.config_path();
    if path.exists() {
        let current = read(paths)?;
        if current == *config {
            return Ok(());
        }
        snapshots::create(paths, &current, ConfigSnapshotReason::BeforeConfigUpdate)?;
        write_contents(paths, config)?;
    } else {
        if let Err(error) = write_contents(paths, config) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        if let Err(error) = snapshots::create(paths, config, ConfigSnapshotReason::Initial) {
            let _ = fs::remove_file(&path);
            return Err(error).context("snapshot initial Stoker config");
        }
    }
    Ok(())
}

pub(super) fn write_contents(paths: &StokerPaths, config: &StokerConfig) -> anyhow::Result<()> {
    let path = paths.config_path();
    let contents = serde_json::to_string_pretty(config)? + "\n";
    fs::write(&path, contents)
        .with_context(|| format!("write Stoker config {}", path.display()))?;
    Ok(())
}
