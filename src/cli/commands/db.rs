//! SQLite health and recovery commands.

use chrono::{SecondsFormat, Utc};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::PathBuf;
use uuid::Uuid;

use crate::{StokerPaths, Store};

use super::super::print_success;

pub(crate) enum DatabaseOperation {
    Check {
        integrity: bool,
    },
    Backup {
        destination: Option<PathBuf>,
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
            let destination = destination.unwrap_or(default_backup_path(paths)?);
            if let Some(parent) = destination.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
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

fn default_backup_path(paths: &StokerPaths) -> anyhow::Result<PathBuf> {
    let directory = paths.root.join("backups");
    std::fs::create_dir_all(&directory)?;
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let timestamp = timestamp.replace(['-', ':'], "");
    Ok(directory.join(format!("stoker-{timestamp}-{}.sqlite", Uuid::new_v4())))
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

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::fs;
    use tempfile::TempDir;

    fn paths(root: &std::path::Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    #[test]
    fn check_backup_and_restore_commands_cover_default_and_explicit_destinations() {
        let directory = TempDir::new().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();

        database(&paths, DatabaseOperation::Check { integrity: false }).unwrap();
        database(&paths, DatabaseOperation::Check { integrity: true }).unwrap();

        let explicit = directory.path().join("explicit.sqlite");
        database(
            &paths,
            DatabaseOperation::Backup {
                destination: Some(explicit.clone()),
            },
        )
        .unwrap();
        assert!(explicit.is_file());

        database(&paths, DatabaseOperation::Backup { destination: None }).unwrap();
        let backups = fs::read_dir(directory.path().join("backups"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        assert!(
            backups[0]
                .path()
                .extension()
                .is_some_and(|ext| ext == "sqlite")
        );

        let restore_error = database(
            &paths,
            DatabaseOperation::Restore {
                source: explicit.clone(),
                yes: false,
            },
        )
        .unwrap_err();
        assert!(restore_error.to_string().contains("requires --yes"));
        database(
            &paths,
            DatabaseOperation::Restore {
                source: explicit,
                yes: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn restore_is_rejected_while_the_scheduler_lock_is_held() {
        let directory = TempDir::new().unwrap();
        let paths = paths(directory.path());
        paths.ensure().unwrap();
        let source = directory.path().join("source.sqlite");
        Store::open(&source).unwrap().quick_check().unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&paths.lock)
            .unwrap();
        lock.try_lock_exclusive().unwrap();

        let error = database(&paths, DatabaseOperation::Restore { source, yes: true }).unwrap_err();
        assert!(error.to_string().contains("scheduler is running"));
        lock.unlock().unwrap();
    }

    #[test]
    fn default_backup_path_is_created_under_the_stoker_root() {
        let directory = TempDir::new().unwrap();
        let paths = paths(directory.path());
        let backup = default_backup_path(&paths).unwrap();
        let expected_parent = directory.path().join("backups");
        assert_eq!(backup.parent(), Some(expected_parent.as_path()));
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("stoker-")
        );
        assert!(backup.extension().is_some_and(|ext| ext == "sqlite"));
    }
}
