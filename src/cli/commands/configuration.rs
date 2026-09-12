//! Configuration command orchestration.

use anyhow::Context;
use crossterm::style::Color;

use crate::StokerPaths;
use crate::application::{self, SnapshotReason};
use crate::config::{LogPolicy, RuntimePolicy, resolve_timezone};
use crate::output;
use crate::{Store, StoreError};

use super::super::{
    ConfigCommand, ConfigKey, application_cli_error, print_success, restore_config, select_timezone,
};

pub(crate) fn config(
    paths: &StokerPaths,
    command: ConfigCommand,
    cli_timezone: Option<&str>,
) -> anyhow::Result<()> {
    match command {
        ConfigCommand::Set { key, value } => match key {
            ConfigKey::Timezone => {
                let current = application::configuration::configuration(paths)
                    .map_err(application_cli_error)?;
                let value = match value {
                    Some(value) => value,
                    None => {
                        let Some(value) = select_timezone(current.timezone.as_deref())? else {
                            return Ok(());
                        };
                        value
                    }
                };
                resolve_timezone(paths, Some(&value))?;
                application::configuration::set_timezone(paths, value.clone())
                    .map_err(application_cli_error)?;
                print_success(format!("Set timezone to {value}."));
            }
            key if is_log_key(key) => set_log_policy(paths, key, value)?,
            key => set_runtime_policy(paths, key, value)?,
        },
        ConfigCommand::Show => {
            let current =
                application::configuration::configuration(paths).map_err(application_cli_error)?;
            let store = Store::open(&paths.database)?;
            let policy = store.log_policy()?;
            let runtime = store.runtime_policy()?;
            println!(
                "{}",
                output::paint_bold(
                    "Stoker configuration",
                    Color::Cyan,
                    output::stdout_color_enabled(),
                )
            );
            println!("File: {}", paths.config_path().display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "timezone": current.timezone,
                    "log": policy,
                    "runtime": runtime,
                }))
                .context("format Stoker configuration")?
            );
        }
        ConfigCommand::Get { key } => get_config(paths, key)?,
        ConfigCommand::Unset { key } => match key {
            ConfigKey::Timezone => {
                application::configuration::unset_timezone(paths).map_err(application_cli_error)?;
                print_success("Unset timezone; using operating system timezone.");
            }
            key if is_log_key(key) => unset_log_policy(paths, key)?,
            key => unset_runtime_policy(paths, key)?,
        },
        ConfigCommand::Restore => restore_config(paths, cli_timezone)?,
        ConfigCommand::Snapshot => {
            let snapshot =
                application::configuration::create_snapshot(paths, SnapshotReason::Manual)
                    .map_err(application_cli_error)?;
            print_success(format!(
                "Created configuration snapshot: {}.",
                snapshot.path.display()
            ));
        }
    }
    Ok(())
}

fn set_log_policy(
    paths: &StokerPaths,
    key: ConfigKey,
    value: Option<String>,
) -> anyhow::Result<()> {
    let value = value.ok_or_else(|| {
        anyhow::anyhow!(
            "a value is required for {}; use bytes such as 64MiB or a non-negative count",
            config_key_name(key)
        )
    })?;
    let parsed = parse_config_value(key, &value)?;
    let store = Store::open(&paths.database)?;
    let mut policy = store.log_policy()?;
    assign_policy_value(&mut policy, key, parsed);
    store.set_log_policy(policy).map_err(policy_cli_error)?;
    print_success(format!("Set {} to {}.", config_key_name(key), value));
    Ok(())
}

fn unset_log_policy(paths: &StokerPaths, key: ConfigKey) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let mut policy = store.log_policy()?;
    let defaults = LogPolicy::default();
    assign_policy_value(&mut policy, key, policy_value(defaults, key));
    store.set_log_policy(policy).map_err(policy_cli_error)?;
    print_success(format!(
        "Unset {}; using the built-in default.",
        config_key_name(key)
    ));
    Ok(())
}

fn set_runtime_policy(
    paths: &StokerPaths,
    key: ConfigKey,
    value: Option<String>,
) -> anyhow::Result<()> {
    let value =
        value.ok_or_else(|| anyhow::anyhow!("a value is required for {}", config_key_name(key)))?;
    let parsed = value.trim().parse::<u64>().map_err(|_| {
        anyhow::anyhow!(
            "{} must be a non-negative millisecond integer",
            config_key_name(key)
        )
    })?;
    let store = Store::open(&paths.database)?;
    let mut policy = store.runtime_policy()?;
    assign_runtime_value(&mut policy, key, Some(parsed));
    store
        .set_runtime_policy(policy)
        .map_err(runtime_cli_error)?;
    print_success(format!("Set {} to {} ms.", config_key_name(key), parsed));
    Ok(())
}

fn unset_runtime_policy(paths: &StokerPaths, key: ConfigKey) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let mut policy = store.runtime_policy()?;
    let defaults = RuntimePolicy::default();
    assign_runtime_value(&mut policy, key, runtime_value(defaults, key));
    store
        .set_runtime_policy(policy)
        .map_err(runtime_cli_error)?;
    print_success(format!(
        "Unset {}; using the built-in default.",
        config_key_name(key)
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

fn get_config(paths: &StokerPaths, key: ConfigKey) -> anyhow::Result<()> {
    match key {
        ConfigKey::Timezone => match application::configuration::configuration(paths)
            .map_err(application_cli_error)?
            .timezone
        {
            Some(value) => println!("timezone: {value}"),
            None => println!("timezone: <using operating system timezone>"),
        },
        key => {
            let store = Store::open(&paths.database)?;
            if is_log_key(key) {
                println!(
                    "{}: {}",
                    config_key_name(key),
                    policy_value(store.log_policy()?, key)
                );
            } else {
                match runtime_value(store.runtime_policy()?, key) {
                    Some(value) => println!("{}: {}", config_key_name(key), value),
                    None => println!("{}: <disabled>", config_key_name(key)),
                }
            }
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

fn config_key_name(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::Timezone => "timezone",
        ConfigKey::LogMaxBytesPerJob => "log-max-bytes-per-job",
        ConfigKey::LogSegmentBytes => "log-segment-bytes",
        ConfigKey::LogMaxBytesTotal => "log-max-bytes-total",
        ConfigKey::LogRetentionJobs => "log-retention-jobs",
        ConfigKey::LogDiskReserveBytes => "log-disk-reserve-bytes",
        ConfigKey::TerminationGraceMs => "termination-grace-ms",
        ConfigKey::MaxRuntimeMs => "max-runtime-ms",
        ConfigKey::StartupTimeoutMs => "startup-timeout-ms",
    }
}

fn is_log_key(key: ConfigKey) -> bool {
    matches!(
        key,
        ConfigKey::LogMaxBytesPerJob
            | ConfigKey::LogSegmentBytes
            | ConfigKey::LogMaxBytesTotal
            | ConfigKey::LogRetentionJobs
            | ConfigKey::LogDiskReserveBytes
    )
}

fn policy_value(policy: LogPolicy, key: ConfigKey) -> u64 {
    match key {
        ConfigKey::LogMaxBytesPerJob => policy.max_bytes_per_job,
        ConfigKey::LogSegmentBytes => policy.segment_bytes,
        ConfigKey::LogMaxBytesTotal => policy.max_bytes_total,
        ConfigKey::LogRetentionJobs => policy.retention_jobs,
        ConfigKey::LogDiskReserveBytes => policy.disk_reserve_bytes,
        ConfigKey::Timezone => unreachable!("timezone is not a log policy value"),
        ConfigKey::TerminationGraceMs | ConfigKey::MaxRuntimeMs | ConfigKey::StartupTimeoutMs => {
            unreachable!("runtime policy key passed to log policy")
        }
    }
}

fn assign_policy_value(policy: &mut LogPolicy, key: ConfigKey, value: u64) {
    match key {
        ConfigKey::LogMaxBytesPerJob => policy.max_bytes_per_job = value,
        ConfigKey::LogSegmentBytes => policy.segment_bytes = value,
        ConfigKey::LogMaxBytesTotal => policy.max_bytes_total = value,
        ConfigKey::LogRetentionJobs => policy.retention_jobs = value,
        ConfigKey::LogDiskReserveBytes => policy.disk_reserve_bytes = value,
        ConfigKey::Timezone => unreachable!("timezone is not a log policy value"),
        ConfigKey::TerminationGraceMs | ConfigKey::MaxRuntimeMs | ConfigKey::StartupTimeoutMs => {
            unreachable!("runtime policy key passed to log policy")
        }
    }
}

fn runtime_value(policy: RuntimePolicy, key: ConfigKey) -> Option<u64> {
    match key {
        ConfigKey::TerminationGraceMs => Some(policy.termination_grace_ms),
        ConfigKey::MaxRuntimeMs => policy.max_runtime_ms,
        ConfigKey::StartupTimeoutMs => Some(policy.startup_timeout_ms),
        _ => None,
    }
}

fn assign_runtime_value(policy: &mut RuntimePolicy, key: ConfigKey, value: Option<u64>) {
    match key {
        ConfigKey::TerminationGraceMs => policy.termination_grace_ms = value.unwrap_or_default(),
        ConfigKey::MaxRuntimeMs => policy.max_runtime_ms = value,
        ConfigKey::StartupTimeoutMs => policy.startup_timeout_ms = value.unwrap_or_default(),
        _ => unreachable!("not a runtime policy key"),
    }
}

fn parse_config_value(key: ConfigKey, value: &str) -> anyhow::Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{} cannot be empty", config_key_name(key));
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (digits, suffix) = value.split_at(split);
    let number = digits
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("{} must be a non-negative integer", config_key_name(key)))?;
    if matches!(key, ConfigKey::LogRetentionJobs) {
        if !suffix.is_empty() {
            anyhow::bail!(
                "{} is a count and does not accept a byte suffix",
                config_key_name(key)
            );
        }
        return Ok(number);
    }
    if matches!(
        key,
        ConfigKey::TerminationGraceMs | ConfigKey::MaxRuntimeMs | ConfigKey::StartupTimeoutMs
    ) {
        if !suffix.is_empty() {
            anyhow::bail!(
                "{} expects milliseconds without a suffix",
                config_key_name(key)
            );
        }
        return Ok(number);
    }
    let multiplier = match suffix.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" => 1_000,
        "kib" => 1_024,
        "mb" => 1_000_000,
        "mib" => 1_048_576,
        "gb" => 1_000_000_000,
        "gib" => 1_073_741_824,
        "tb" => 1_000_000_000_000,
        "tib" => 1_099_511_627_776,
        _ => anyhow::bail!("unsupported byte suffix '{suffix}'"),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("{} is too large", config_key_name(key)))
}
