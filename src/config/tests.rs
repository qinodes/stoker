use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};

use super::*;

fn test_paths(root: &Path) -> StokerPaths {
    StokerPaths {
        root: root.to_path_buf(),
        database: root.join("stoker.db"),
        runs: root.join("runs"),
        lock: root.join("stoker.lock"),
        endpoint: root.join("stoker.sock"),
    }
}

#[test]
fn path_construction_has_no_filesystem_side_effects_and_bootstrap_is_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let paths = test_paths(&root);
    assert!(!root.exists());
    assert_eq!(paths.config_path(), root.join("config.json"));
    assert_eq!(paths.snapshot_dir(), root.join("snapshot"));
    assert_eq!(paths.service_log(), root.join("service.log"));
    assert_eq!(paths.ui_metadata(), root.join("ui.json"));
    assert_eq!(paths.ui_log(), root.join("ui.log"));
    assert!(!paths.ipc_endpoint().is_empty());
    assert!(paths.list_config_snapshots().unwrap().is_empty());

    paths.ensure().unwrap();
    assert!(root.is_dir());
    assert!(paths.runs.is_dir());
    assert!(paths.config_path().is_file());
}

#[test]
fn config_initialization_uses_system_timezone_and_preserves_existing_config() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    paths.ensure().unwrap();
    assert!(paths.read_config().unwrap().timezone.is_some());

    let configured = StokerConfig {
        timezone: Some("Asia/Tokyo".into()),
    };
    paths.write_config(&configured).unwrap();
    paths.ensure().unwrap();
    assert_eq!(paths.read_config().unwrap(), configured);
}

#[test]
fn timezone_resolution_covers_cli_config_system_and_invalid_values() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    paths
        .write_config(&StokerConfig {
            timezone: Some("Asia/Taipei".into()),
        })
        .unwrap();

    let from_config = resolve_timezone(&paths, None).unwrap();
    assert_eq!(from_config.name, "Asia/Taipei");
    assert_eq!(from_config.source, TimezoneSource::Config);
    let from_cli = resolve_timezone(&paths, Some("Asia/Tokyo")).unwrap();
    assert_eq!(from_cli.source, TimezoneSource::Cli);
    assert_eq!(
        from_cli.format(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
        "2026-01-01T09:00:00.000+09:00"
    );
    assert!(resolve_timezone(&paths, Some("Not/AZone")).is_err());

    let empty = test_paths(directory.path().join("empty").as_path());
    let from_system = resolve_timezone(&empty, None).unwrap();
    assert_eq!(from_system.source, TimezoneSource::System);
    assert!(!from_system.name.is_empty());
}

#[test]
fn config_updates_snapshot_previous_values_and_skip_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    let first = StokerConfig {
        timezone: Some("Asia/Tokyo".into()),
    };
    let second = StokerConfig {
        timezone: Some("UTC".into()),
    };

    paths.write_config(&first).unwrap();
    assert_eq!(paths.list_config_snapshots().unwrap().len(), 1);
    paths.write_config(&first).unwrap();
    assert_eq!(paths.list_config_snapshots().unwrap().len(), 1);
    paths.write_config(&second).unwrap();

    let previous = paths
        .list_config_snapshots()
        .unwrap()
        .into_iter()
        .find_map(|entry| match entry {
            ConfigSnapshotEntry::Valid(file)
                if file.snapshot.reason == ConfigSnapshotReason::BeforeConfigUpdate =>
            {
                Some(file.snapshot.config)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(previous, first);
}

#[test]
fn restoring_a_snapshot_preserves_current_config_and_noop_is_stable() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    let first = StokerConfig {
        timezone: Some("Asia/Tokyo".into()),
    };
    let second = StokerConfig {
        timezone: Some("UTC".into()),
    };
    paths.write_config(&first).unwrap();
    paths.write_config(&second).unwrap();
    let target = paths
        .list_config_snapshots()
        .unwrap()
        .into_iter()
        .find_map(|entry| match entry {
            ConfigSnapshotEntry::Valid(file)
                if file.snapshot.reason == ConfigSnapshotReason::BeforeConfigUpdate =>
            {
                Some(file.snapshot)
            }
            _ => None,
        })
        .unwrap();

    assert!(paths.restore_config_snapshot(&target).unwrap());
    assert_eq!(paths.read_config().unwrap(), first);
    assert!(paths.list_config_snapshots().unwrap().iter().any(|entry| {
        matches!(entry, ConfigSnapshotEntry::Valid(file) if file.snapshot.reason == ConfigSnapshotReason::BeforeRestore)
    }));
    assert!(!paths.restore_config_snapshot(&target).unwrap());
}

#[test]
fn snapshot_listing_reports_invalid_versions_and_sorts_deterministically() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    let valid_path = paths
        .create_config_snapshot(&StokerConfig::default(), ConfigSnapshotReason::Manual)
        .unwrap();
    fs::write(paths.snapshot_dir().join("broken-a.json"), "not-json").unwrap();
    fs::write(paths.snapshot_dir().join("broken-b.json"), "not-json").unwrap();
    fs::write(
        paths.snapshot_dir().join("version.json"),
        serde_json::json!({
            "snapshot_version": 99,
            "created_at": "2026-01-01T00:00:00Z",
            "reason": "manual",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();
    fs::write(paths.snapshot_dir().join("ignored.txt"), "ignored").unwrap();

    let entries = paths.list_config_snapshots().unwrap();
    assert_eq!(entries.len(), 4);
    assert!(
        matches!(entries.first(), Some(ConfigSnapshotEntry::Valid(file)) if file.path == valid_path)
    );
    let invalid_paths: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            ConfigSnapshotEntry::Invalid { path, .. } => path.file_name(),
            ConfigSnapshotEntry::Valid(_) => None,
        })
        .collect();
    assert_eq!(
        invalid_paths,
        ["version.json", "broken-b.json", "broken-a.json"]
    );
    assert!(entries.iter().any(|entry| matches!(
        entry,
        ConfigSnapshotEntry::Invalid { path, error }
            if path.ends_with("version.json") && error.contains("unsupported snapshot version")
    )));
}

#[test]
fn manual_snapshots_are_always_created_and_reasons_are_stable() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    let config = StokerConfig {
        timezone: Some("UTC".into()),
    };
    paths.write_config(&config).unwrap();
    paths
        .create_config_snapshot(&config, ConfigSnapshotReason::Manual)
        .unwrap();
    paths
        .create_config_snapshot(&config, ConfigSnapshotReason::Manual)
        .unwrap();
    assert_eq!(paths.list_config_snapshots().unwrap().len(), 3);

    assert_eq!(ConfigSnapshotReason::Initial.to_string(), "initial");
    assert_eq!(
        ConfigSnapshotReason::BeforeConfigUpdate.to_string(),
        "before config update"
    );
    assert_eq!(
        ConfigSnapshotReason::BeforeRestore.to_string(),
        "before restore"
    );
    assert_eq!(ConfigSnapshotReason::Manual.to_string(), "manual");
}

#[test]
fn missing_malformed_and_invalid_config_paths_are_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    assert_eq!(paths.read_config().unwrap(), StokerConfig::default());

    fs::write(paths.config_path(), "{not-json").unwrap();
    assert!(
        paths
            .read_config()
            .unwrap_err()
            .to_string()
            .contains("parse Stoker config")
    );
    fs::remove_file(paths.config_path()).unwrap();
    assert!(
        paths
            .write_config(&StokerConfig {
                timezone: Some("Not/AZone".into()),
            })
            .unwrap_err()
            .to_string()
            .contains("unknown timezone")
    );
}

#[test]
fn repository_and_workspace_report_unusable_root_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root-file");
    fs::write(&root, "not a directory").unwrap();
    let paths = test_paths(&root);

    assert!(
        paths
            .write_config(&StokerConfig::default())
            .unwrap_err()
            .to_string()
            .contains("write Stoker config")
    );
    assert!(super::workspace::initialize_config(&paths).is_err());
    assert!(paths.ensure().is_err());
}

#[test]
fn snapshot_failures_do_not_leave_partial_config_files() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    fs::write(paths.snapshot_dir(), "blocks snapshot directory").unwrap();
    let config = StokerConfig {
        timezone: Some("UTC".into()),
    };

    assert!(
        paths
            .create_config_snapshot(&config, ConfigSnapshotReason::Manual)
            .is_err()
    );
    assert!(paths.list_config_snapshots().is_err());
    assert!(paths.write_config(&config).is_err());
    assert!(!paths.config_path().exists());
}

#[test]
fn snapshot_operations_reject_invalid_config_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    let invalid = StokerConfig {
        timezone: Some("Not/AZone".into()),
    };
    assert!(
        paths
            .create_config_snapshot(&invalid, ConfigSnapshotReason::Manual)
            .is_err()
    );
    assert!(
        paths
            .restore_config_snapshot(&ConfigSnapshot {
                snapshot_version: 1,
                created_at: Utc::now(),
                reason: ConfigSnapshotReason::Manual,
                config: invalid,
            })
            .is_err()
    );
    assert!(!paths.snapshot_dir().exists());
}

#[test]
fn snapshots_without_a_version_use_the_compatible_default() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    fs::create_dir_all(paths.snapshot_dir()).unwrap();
    let valid_path = paths.snapshot_dir().join("valid.json");
    fs::write(
        &valid_path,
        serde_json::json!({
            "created_at": "2026-01-01T00:00:00Z",
            "reason": "manual",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();

    assert!(matches!(
        paths.list_config_snapshots().unwrap().as_slice(),
        [ConfigSnapshotEntry::Valid(file)] if file.path == valid_path && file.snapshot.snapshot_version == 1
    ));
}

#[test]
fn log_policy_validation_rejects_zero_and_inconsistent_limits() {
    let defaults = LogPolicy::default();

    let mut invalid = defaults;
    invalid.max_bytes_per_job = 0;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "log max bytes per job must be greater than zero"
    );

    invalid = defaults;
    invalid.segment_bytes = 0;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "log segment bytes must be greater than zero"
    );

    invalid = defaults;
    invalid.segment_bytes = defaults.max_bytes_per_job / 2 + 1;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "log segment bytes cannot exceed half of the shared per-job log limit"
    );

    invalid = defaults;
    invalid.max_bytes_total = defaults.max_bytes_per_job - 1;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "global log limit cannot be smaller than the per-job log limit"
    );

    invalid = defaults;
    invalid.disk_reserve_bytes = 0;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "log disk reserve bytes must be greater than zero"
    );
}

#[test]
fn runtime_policy_validation_rejects_zero_values_but_allows_disabled_limit() {
    let defaults = RuntimePolicy::default();
    assert!(defaults.validate().is_ok());

    let mut invalid = defaults;
    invalid.termination_grace_ms = 0;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "termination grace must be greater than zero"
    );

    invalid = defaults;
    invalid.startup_timeout_ms = 0;
    assert_eq!(
        invalid.validate().unwrap_err(),
        "startup timeout must be greater than zero"
    );

    invalid = defaults;
    invalid.max_runtime_ms = Some(0);
    assert_eq!(
        invalid.validate().unwrap_err(),
        "maximum runtime must be greater than zero when set"
    );

    assert_eq!(defaults.max_runtime_ms, None);
}
