//! Scheduler execution and log policy command orchestration.

use anyhow::Context;
use crossterm::style::Color;

use crate::StokerPaths;
use crate::config::{LogPolicy, POLICY_MB_BYTES, RuntimePolicy};
use crate::output;
use crate::{Store, StoreError};

use super::super::{PolicyCommand, PolicyKey, print_success};

pub(crate) fn policy(paths: &StokerPaths, command: PolicyCommand) -> anyhow::Result<()> {
    match command {
        PolicyCommand::Set { key, value } => {
            if is_log_key(key) {
                set_log_policy(paths, key, value)?;
            } else {
                set_runtime_policy(paths, key, value)?;
            }
        }
        PolicyCommand::Show => {
            let store = Store::open(&paths.database)?;
            let log = store.log_policy()?;
            let runtime = store.runtime_policy()?;
            println!(
                "{}",
                output::paint_bold(
                    "Stoker scheduler policy",
                    Color::Cyan,
                    output::stdout_color_enabled(),
                )
            );
            println!("Database: {}", paths.database.display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "log": {
                        "max_bytes_per_job": display_policy_value(log, PolicyKey::LogMaxBytesPerJob),
                        "segment_bytes": display_policy_value(log, PolicyKey::LogSegmentBytes),
                        "max_bytes_total": display_policy_value(log, PolicyKey::LogMaxBytesTotal),
                        "retention_jobs": display_policy_value(log, PolicyKey::LogRetentionJobs),
                        "disk_reserve_bytes": display_policy_value(log, PolicyKey::LogDiskReserveBytes),
                    },
                    "runtime": runtime,
                }))
                .context("format Stoker scheduler policy")?
            );
        }
        PolicyCommand::Get { key } => get_policy(paths, key)?,
        PolicyCommand::Unset { key } => {
            if is_log_key(key) {
                unset_log_policy(paths, key)?;
            } else {
                unset_runtime_policy(paths, key)?;
            }
        }
    }
    Ok(())
}

fn set_log_policy(
    paths: &StokerPaths,
    key: PolicyKey,
    value: Option<String>,
) -> anyhow::Result<()> {
    let value = value.ok_or_else(|| {
        if is_byte_key(key) {
            anyhow::anyhow!(
                "a value is required for {}; use a whole number of MB such as 64",
                policy_key_name(key)
            )
        } else {
            anyhow::anyhow!(
                "a value is required for {}; use a non-negative whole number",
                policy_key_name(key)
            )
        }
    })?;
    let parsed = parse_policy_value(key, &value)?;
    let store = Store::open(&paths.database)?;
    let mut policy = store.log_policy()?;
    assign_policy_value(&mut policy, key, parsed);
    store.set_log_policy(policy).map_err(policy_cli_error)?;
    print_success(format!("Set {} to {}.", policy_key_name(key), value));
    Ok(())
}

fn unset_log_policy(paths: &StokerPaths, key: PolicyKey) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let mut policy = store.log_policy()?;
    let defaults = LogPolicy::default();
    assign_policy_value(&mut policy, key, policy_value(defaults, key));
    store.set_log_policy(policy).map_err(policy_cli_error)?;
    print_success(format!(
        "Unset {}; using the built-in default.",
        policy_key_name(key)
    ));
    Ok(())
}

fn set_runtime_policy(
    paths: &StokerPaths,
    key: PolicyKey,
    value: Option<String>,
) -> anyhow::Result<()> {
    let value =
        value.ok_or_else(|| anyhow::anyhow!("a value is required for {}", policy_key_name(key)))?;
    let parsed = value.trim().parse::<u64>().map_err(|_| {
        anyhow::anyhow!(
            "{} must be a non-negative millisecond integer",
            policy_key_name(key)
        )
    })?;
    let store = Store::open(&paths.database)?;
    let mut policy = store.runtime_policy()?;
    assign_runtime_value(&mut policy, key, Some(parsed));
    store
        .set_runtime_policy(policy)
        .map_err(runtime_cli_error)?;
    print_success(format!("Set {} to {} ms.", policy_key_name(key), parsed));
    Ok(())
}

fn unset_runtime_policy(paths: &StokerPaths, key: PolicyKey) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let mut policy = store.runtime_policy()?;
    let defaults = RuntimePolicy::default();
    assign_runtime_value(&mut policy, key, runtime_value(defaults, key));
    store
        .set_runtime_policy(policy)
        .map_err(runtime_cli_error)?;
    print_success(format!(
        "Unset {}; using the built-in default.",
        policy_key_name(key)
    ));
    Ok(())
}

fn runtime_cli_error(error: StoreError) -> anyhow::Error {
    match error {
        StoreError::QueueUnlocked => anyhow::anyhow!(
            "cannot change runtime policy while the queue is unlocked; run `stoker queue lock` first"
        ),
        StoreError::ActiveJob { id, state } => anyhow::anyhow!(
            "cannot change runtime policy while job {id} is {state}; wait for it to finish"
        ),
        other => anyhow::Error::new(other),
    }
}

fn get_policy(paths: &StokerPaths, key: PolicyKey) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    if is_log_key(key) {
        println!(
            "{}: {}",
            policy_key_name(key),
            display_policy_value(store.log_policy()?, key)
        );
    } else {
        match runtime_value(store.runtime_policy()?, key) {
            Some(value) => println!("{}: {}", policy_key_name(key), value),
            None => println!("{}: <disabled>", policy_key_name(key)),
        }
    }
    Ok(())
}

fn policy_cli_error(error: StoreError) -> anyhow::Error {
    match error {
        StoreError::QueueUnlocked => anyhow::anyhow!(
            "cannot change log capacity while the queue is unlocked; run `stoker queue lock` first"
        ),
        StoreError::ActiveJob { id, state } => anyhow::anyhow!(
            "cannot change log capacity while job {id} is {state}; keep the queue locked and wait for the active job to finish"
        ),
        other => anyhow::Error::new(other),
    }
}

fn policy_key_name(key: PolicyKey) -> &'static str {
    match key {
        PolicyKey::LogMaxBytesPerJob => "log-max-bytes-per-job",
        PolicyKey::LogSegmentBytes => "log-segment-bytes",
        PolicyKey::LogMaxBytesTotal => "log-max-bytes-total",
        PolicyKey::LogRetentionJobs => "log-retention-jobs",
        PolicyKey::LogDiskReserveBytes => "log-disk-reserve-bytes",
        PolicyKey::TerminationGraceMs => "termination-grace-ms",
        PolicyKey::MaxRuntimeMs => "max-runtime-ms",
        PolicyKey::StartupTimeoutMs => "startup-timeout-ms",
    }
}

fn is_log_key(key: PolicyKey) -> bool {
    matches!(
        key,
        PolicyKey::LogMaxBytesPerJob
            | PolicyKey::LogSegmentBytes
            | PolicyKey::LogMaxBytesTotal
            | PolicyKey::LogRetentionJobs
            | PolicyKey::LogDiskReserveBytes
    )
}

fn policy_value(policy: LogPolicy, key: PolicyKey) -> u64 {
    match key {
        PolicyKey::LogMaxBytesPerJob => policy.max_bytes_per_job,
        PolicyKey::LogSegmentBytes => policy.segment_bytes,
        PolicyKey::LogMaxBytesTotal => policy.max_bytes_total,
        PolicyKey::LogRetentionJobs => policy.retention_jobs,
        PolicyKey::LogDiskReserveBytes => policy.disk_reserve_bytes,
        PolicyKey::TerminationGraceMs | PolicyKey::MaxRuntimeMs | PolicyKey::StartupTimeoutMs => {
            unreachable!("runtime policy key passed to log policy")
        }
    }
}

fn display_policy_value(policy: LogPolicy, key: PolicyKey) -> u64 {
    let value = policy_value(policy, key);
    if is_byte_key(key) {
        value.div_ceil(POLICY_MB_BYTES)
    } else {
        value
    }
}

fn is_byte_key(key: PolicyKey) -> bool {
    matches!(
        key,
        PolicyKey::LogMaxBytesPerJob
            | PolicyKey::LogSegmentBytes
            | PolicyKey::LogMaxBytesTotal
            | PolicyKey::LogDiskReserveBytes
    )
}

fn assign_policy_value(policy: &mut LogPolicy, key: PolicyKey, value: u64) {
    match key {
        PolicyKey::LogMaxBytesPerJob => policy.max_bytes_per_job = value,
        PolicyKey::LogSegmentBytes => policy.segment_bytes = value,
        PolicyKey::LogMaxBytesTotal => policy.max_bytes_total = value,
        PolicyKey::LogRetentionJobs => policy.retention_jobs = value,
        PolicyKey::LogDiskReserveBytes => policy.disk_reserve_bytes = value,
        PolicyKey::TerminationGraceMs | PolicyKey::MaxRuntimeMs | PolicyKey::StartupTimeoutMs => {
            unreachable!("runtime policy key passed to log policy")
        }
    }
}

fn runtime_value(policy: RuntimePolicy, key: PolicyKey) -> Option<u64> {
    match key {
        PolicyKey::TerminationGraceMs => Some(policy.termination_grace_ms),
        PolicyKey::MaxRuntimeMs => policy.max_runtime_ms,
        PolicyKey::StartupTimeoutMs => Some(policy.startup_timeout_ms),
        _ => None,
    }
}

fn assign_runtime_value(policy: &mut RuntimePolicy, key: PolicyKey, value: Option<u64>) {
    match key {
        PolicyKey::TerminationGraceMs => policy.termination_grace_ms = value.unwrap_or_default(),
        PolicyKey::MaxRuntimeMs => policy.max_runtime_ms = value,
        PolicyKey::StartupTimeoutMs => policy.startup_timeout_ms = value.unwrap_or_default(),
        _ => unreachable!("not a runtime policy key"),
    }
}

fn parse_policy_value(key: PolicyKey, value: &str) -> anyhow::Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{} cannot be empty", policy_key_name(key));
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (digits, suffix) = value.split_at(split);
    let number = digits
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("{} must be a non-negative integer", policy_key_name(key)))?;
    if matches!(key, PolicyKey::LogRetentionJobs) {
        if !suffix.is_empty() {
            anyhow::bail!(
                "{} is a count and does not accept a byte suffix",
                policy_key_name(key)
            );
        }
        return Ok(number);
    }
    if matches!(
        key,
        PolicyKey::TerminationGraceMs | PolicyKey::MaxRuntimeMs | PolicyKey::StartupTimeoutMs
    ) {
        if !suffix.is_empty() {
            anyhow::bail!(
                "{} expects milliseconds without a suffix",
                policy_key_name(key)
            );
        }
        return Ok(number);
    }
    if !is_byte_key(key) {
        anyhow::bail!("{} is not a supported policy value", policy_key_name(key));
    }
    if !suffix.is_empty() {
        anyhow::bail!(
            "{} accepts only a whole number of MB without a unit suffix",
            policy_key_name(key)
        );
    }
    number
        .checked_mul(POLICY_MB_BYTES)
        .ok_or_else(|| anyhow::anyhow!("{} is too large", policy_key_name(key)))
}
