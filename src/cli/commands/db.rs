//! SQLite health and recovery commands.

use fs2::FileExt;
use std::fs::OpenOptions;

use crate::{StokerPaths, Store};

use super::super::print_success;

pub(crate) enum DatabaseOperation {
    Check {
        integrity: bool,
    },
    Backup {
        destination: std::path::PathBuf,
    },
    Restore {
        source: std::path::PathBuf,
        yes: bool,
    },
}

pub(crate) fn database(paths: &StokerPaths, command: DatabaseOperation) -> anyhow::Result<()> {
    match command {
        DatabaseOperation::Check { integrity } => {
            let store = Store::open(&paths.database)?;
            if integrity {
                store.integrity_check()?;
                print_success("SQLite integrity_check passed.");
            } else {
                store.quick_check()?;
                print_success("SQLite quick_check passed.");
            }
        }
        DatabaseOperation::Backup { destination } => {
            let store = Store::open(&paths.database)?;
            store.backup_to(&destination)?;
            print_success(format!("Created SQLite backup: {}.", destination.display()));
        }
        DatabaseOperation::Restore { source, yes } => {
            if !yes {
                anyhow::bail!(
                    "database restore replaces {} and requires --yes",
                    paths.database.display()
                );
            }
            let source_store = Store::open(&source)?;
            source_store.quick_check()?;
            ensure_service_stopped(paths)?;
            std::fs::copy(&source, &paths.database)?;
            print_success(format!(
                "Restored SQLite database from {}.",
                source.display()
            ));
        }
    }
    Ok(())
}

fn ensure_service_stopped(paths: &StokerPaths) -> anyhow::Result<()> {
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)?;
    lock.try_lock_exclusive().map_err(|error| {
        anyhow::anyhow!(
            "cannot restore database while the scheduler is running ({}): {error}",
            paths.lock.display()
        )
    })?;
    Ok(())
}
