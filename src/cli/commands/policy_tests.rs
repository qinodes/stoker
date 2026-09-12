use super::*;

const LOG_KEYS: [PolicyKey; 5] = [
    PolicyKey::LogMaxBytesPerJob,
    PolicyKey::LogSegmentBytes,
    PolicyKey::LogMaxBytesTotal,
    PolicyKey::LogRetentionJobs,
    PolicyKey::LogDiskReserveBytes,
];

const RUNTIME_KEYS: [PolicyKey; 3] = [
    PolicyKey::TerminationGraceMs,
    PolicyKey::MaxRuntimeMs,
    PolicyKey::StartupTimeoutMs,
];

#[test]
fn policy_key_helpers_cover_all_user_keys() {
    let expected_log_names = [
        "log-max-bytes-per-job",
        "log-segment-bytes",
        "log-max-bytes-total",
        "log-retention-jobs",
        "log-disk-reserve-bytes",
    ];
    for (key, expected_name) in LOG_KEYS.into_iter().zip(expected_log_names) {
        assert!(is_log_key(key));
        assert_eq!(policy_key_name(key), expected_name);
        assert_eq!(runtime_value(RuntimePolicy::default(), key), None);
        assert_eq!(
            is_byte_key(key),
            !matches!(key, PolicyKey::LogRetentionJobs)
        );
    }
    for key in RUNTIME_KEYS {
        assert!(!is_log_key(key));
        assert!(!is_byte_key(key));
        assert!(
            runtime_value(RuntimePolicy::default(), key).is_some()
                || matches!(key, PolicyKey::MaxRuntimeMs)
        );
    }

    let mut log = LogPolicy::default();
    for (index, key) in LOG_KEYS.into_iter().enumerate() {
        assign_policy_value(&mut log, key, (index as u64 + 1) * POLICY_MB_BYTES);
        assert_eq!(policy_value(log, key), (index as u64 + 1) * POLICY_MB_BYTES);
        let expected_display = if matches!(key, PolicyKey::LogRetentionJobs) {
            (index as u64 + 1) * POLICY_MB_BYTES
        } else {
            index as u64 + 1
        };
        assert_eq!(display_policy_value(log, key), expected_display);
    }
    log.max_bytes_per_job = POLICY_MB_BYTES + 1;
    assert_eq!(display_policy_value(log, PolicyKey::LogMaxBytesPerJob), 2);

    let mut runtime = RuntimePolicy::default();
    assign_runtime_value(&mut runtime, PolicyKey::TerminationGraceMs, Some(123));
    assign_runtime_value(&mut runtime, PolicyKey::MaxRuntimeMs, Some(456));
    assign_runtime_value(&mut runtime, PolicyKey::StartupTimeoutMs, Some(789));
    assert_eq!(
        runtime_value(runtime, PolicyKey::TerminationGraceMs),
        Some(123)
    );
    assert_eq!(runtime_value(runtime, PolicyKey::MaxRuntimeMs), Some(456));
    assert_eq!(
        runtime_value(runtime, PolicyKey::StartupTimeoutMs),
        Some(789)
    );
    assign_runtime_value(&mut runtime, PolicyKey::MaxRuntimeMs, None);
    assert_eq!(runtime_value(runtime, PolicyKey::MaxRuntimeMs), None);
}

#[test]
fn policy_value_parser_enforces_units_and_overflow() {
    assert_eq!(
        parse_policy_value(PolicyKey::LogMaxBytesPerJob, " 2 ").unwrap(),
        2 * POLICY_MB_BYTES
    );
    assert_eq!(
        parse_policy_value(PolicyKey::LogRetentionJobs, "0").unwrap(),
        0
    );
    assert_eq!(
        parse_policy_value(PolicyKey::TerminationGraceMs, "500").unwrap(),
        500
    );

    for (key, value, expected) in [
        (PolicyKey::LogMaxBytesPerJob, "", "cannot be empty"),
        (
            PolicyKey::LogMaxBytesPerJob,
            "-1",
            "must be a non-negative integer",
        ),
        (
            PolicyKey::LogMaxBytesPerJob,
            "1MiB",
            "without a unit suffix",
        ),
        (
            PolicyKey::LogRetentionJobs,
            "1MB",
            "does not accept a byte suffix",
        ),
        (PolicyKey::TerminationGraceMs, "1ms", "without a suffix"),
        (
            PolicyKey::LogMaxBytesPerJob,
            "18446744073709551615",
            "too large",
        ),
    ] {
        let error = parse_policy_value(key, value).unwrap_err().to_string();
        assert!(error.contains(expected), "{value:?}: {error}");
    }
}

#[test]
fn policy_error_messages_preserve_queue_and_active_job_context() {
    let queue_error = policy_cli_error(StoreError::QueueUnlocked).to_string();
    assert!(queue_error.contains("log capacity"));
    let runtime_queue_error = runtime_cli_error(StoreError::QueueUnlocked).to_string();
    assert!(runtime_queue_error.contains("runtime policy"));

    let id = uuid::Uuid::nil();
    let active = StoreError::ActiveJob {
        id,
        state: crate::domain::JobState::Running,
    };
    assert!(
        policy_cli_error(active)
            .to_string()
            .contains(&id.to_string())
    );
    let active = StoreError::ActiveJob {
        id,
        state: crate::domain::JobState::Cancelling,
    };
    assert!(runtime_cli_error(active).to_string().contains("CANCELLING"));
}

#[test]
fn policy_commands_cover_all_log_keys_and_queue_gated_failures() {
    let directory = tempfile::tempdir().unwrap();
    let paths = crate::config::StokerPaths {
        root: directory.path().to_path_buf(),
        database: directory.path().join("stoker.db"),
        runs: directory.path().join("runs"),
        lock: directory.path().join("stoker.lock"),
        endpoint: directory.path().join("stoker.sock"),
    };
    paths.ensure().unwrap();

    let missing_bytes = policy(
        &paths,
        PolicyCommand::Set {
            key: PolicyKey::LogMaxBytesPerJob,
            value: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(missing_bytes.contains("whole number of MB"));
    let missing_count = policy(
        &paths,
        PolicyCommand::Set {
            key: PolicyKey::LogRetentionJobs,
            value: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(missing_count.contains("non-negative whole number"));
    let parse_error = policy(
        &paths,
        PolicyCommand::Set {
            key: PolicyKey::LogMaxBytesPerJob,
            value: Some("2MiB".into()),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(parse_error.contains("without a unit suffix"));

    let log_values = [
        (PolicyKey::LogMaxBytesPerJob, "64"),
        (PolicyKey::LogSegmentBytes, "1"),
        (PolicyKey::LogMaxBytesTotal, "128"),
        (PolicyKey::LogRetentionJobs, "5"),
        (PolicyKey::LogDiskReserveBytes, "1"),
    ];
    for (key, value) in log_values {
        let error = policy(
            &paths,
            PolicyCommand::Set {
                key,
                value: Some(value.into()),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("queue is unlocked"));
    }

    let store = Store::open(&paths.database).unwrap();
    store.lock_queue().unwrap();
    for (key, value) in log_values {
        policy(
            &paths,
            PolicyCommand::Set {
                key,
                value: Some(value.into()),
            },
        )
        .unwrap();
    }
    for key in log_values.into_iter().map(|(key, _)| key) {
        policy(&paths, PolicyCommand::Get { key }).unwrap();
        policy(&paths, PolicyCommand::Unset { key }).unwrap();
    }
}

#[test]
fn policy_commands_cover_runtime_input_errors_updates_and_unset() {
    let directory = tempfile::tempdir().unwrap();
    let paths = crate::config::StokerPaths {
        root: directory.path().to_path_buf(),
        database: directory.path().join("stoker.db"),
        runs: directory.path().join("runs"),
        lock: directory.path().join("stoker.lock"),
        endpoint: directory.path().join("stoker.sock"),
    };
    paths.ensure().unwrap();

    for key in [
        PolicyKey::TerminationGraceMs,
        PolicyKey::MaxRuntimeMs,
        PolicyKey::StartupTimeoutMs,
    ] {
        let error = policy(&paths, PolicyCommand::Set { key, value: None }).unwrap_err();
        assert!(error.to_string().contains("value is required"));
    }
    let invalid = policy(
        &paths,
        PolicyCommand::Set {
            key: PolicyKey::TerminationGraceMs,
            value: Some("1ms".into()),
        },
    )
    .unwrap_err();
    assert!(invalid.to_string().contains("millisecond integer"));

    let store = Store::open(&paths.database).unwrap();
    store.lock_queue().unwrap();
    for (key, value) in [
        (PolicyKey::TerminationGraceMs, "750"),
        (PolicyKey::MaxRuntimeMs, "1000"),
        (PolicyKey::StartupTimeoutMs, "2000"),
    ] {
        policy(
            &paths,
            PolicyCommand::Set {
                key,
                value: Some(value.into()),
            },
        )
        .unwrap();
        policy(&paths, PolicyCommand::Get { key }).unwrap();
    }
    policy(
        &paths,
        PolicyCommand::Unset {
            key: PolicyKey::MaxRuntimeMs,
        },
    )
    .unwrap();
    policy(
        &paths,
        PolicyCommand::Get {
            key: PolicyKey::MaxRuntimeMs,
        },
    )
    .unwrap();
    policy(
        &paths,
        PolicyCommand::Unset {
            key: PolicyKey::TerminationGraceMs,
        },
    )
    .unwrap();
    policy(
        &paths,
        PolicyCommand::Unset {
            key: PolicyKey::StartupTimeoutMs,
        },
    )
    .unwrap();
    policy(&paths, PolicyCommand::Show).unwrap();
}
