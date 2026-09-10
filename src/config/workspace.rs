use std::fs::{self, OpenOptions};
use std::io::Write;

use anyhow::Context;

use super::model::{ConfigSnapshotReason, StokerConfig};
use super::paths::StokerPaths;
use super::{snapshots, timezone};

impl StokerPaths {
    /// Compatibility entry point; workspace side effects are owned here rather
    /// than by path construction.
    pub fn ensure(&self) -> anyhow::Result<()> {
        ensure(self)
    }
}

pub(super) fn ensure(paths: &StokerPaths) -> anyhow::Result<()> {
    fs::create_dir_all(&paths.root)?;
    fs::create_dir_all(&paths.runs)?;
    initialize_config(paths)?;
    Ok(())
}

pub(super) fn initialize_config(paths: &StokerPaths) -> anyhow::Result<()> {
    let path = paths.config_path();
    if path.exists() {
        return Ok(());
    }

    let Ok(timezone) = timezone::system_timezone_name() else {
        return Ok(());
    };
    let config = StokerConfig {
        timezone: Some(timezone),
    };
    let contents = serde_json::to_string_pretty(&config)? + "\n";
    let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("create Stoker config {}", path.display()));
        }
    };
    if let Err(error) = file.write_all(contents.as_bytes()) {
        let _ = fs::remove_file(&path);
        return Err(error).with_context(|| format!("initialize Stoker config {}", path.display()));
    }
    drop(file);
    if let Err(error) = snapshots::create(paths, &config, ConfigSnapshotReason::Initial) {
        let _ = fs::remove_file(&path);
        return Err(error).context("snapshot initialized Stoker config");
    }
    Ok(())
}
