//! Scheduler status command orchestration.

use crate::application;
use crate::config::{ResolvedTimezone, resolve_timezone};
use crate::log_storage;
use crate::{StokerPaths, Store};

use super::super::presentation::status::{
    print_scheduler_status, print_timezone_status as render_timezone_status,
};
use super::super::{application_cli_error, runtime, scheduler_gateway};

pub(crate) fn status(paths: &StokerPaths) -> anyhow::Result<()> {
    let store = Store::open(&paths.database)?;
    let scheduler = scheduler_gateway(paths);
    let timezone = resolve_timezone(paths, None)?;
    print_timezone_status(paths, &timezone);
    let status = runtime()?
        .block_on(application::queue::queue_status(&store, &scheduler))
        .map_err(application_cli_error)?;
    print_scheduler_status(&status);
    if let Ok(policy) = store.log_policy()
        && let Ok(available) = log_storage::available_space(&paths.runs)
        && available < policy.disk_reserve_bytes
    {
        eprintln!(
            "warning: scheduler is blocking new jobs because available log storage ({available} bytes) is below the configured reserve ({} bytes)",
            policy.disk_reserve_bytes
        );
    }
    Ok(())
}

pub(crate) fn print_timezone_status(paths: &StokerPaths, timezone: &ResolvedTimezone) {
    let config_path = paths.config_path();
    render_timezone_status(&config_path, config_path.exists(), timezone);
}
