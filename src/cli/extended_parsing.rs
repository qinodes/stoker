//! Input validation and schedule parsing for the extended CLI.

use std::collections::BTreeSet;

use anyhow::Result;
use chrono::Utc;

use crate::StokerPaths;
use crate::domain::flow::{
    Dependency, DependencyStatus, ScheduleSpec, parse_daily, parse_every, parse_first_at,
    parse_once, resolve_schedule_timezone,
};

pub(super) fn parse_schedule(
    paths: &StokerPaths,
    once_at: Option<&str>,
    daily: Option<&str>,
    every: Option<&str>,
    first_at: Option<&str>,
    explicit_timezone: Option<&str>,
) -> Result<Option<ScheduleSpec>> {
    let selected = usize::from(once_at.is_some())
        + usize::from(daily.is_some())
        + usize::from(every.is_some());
    if selected > 1 {
        anyhow::bail!("--once-at, --daily, and --every are mutually exclusive");
    }
    if let Some(value) = once_at {
        return Ok(Some(ScheduleSpec::Once {
            at: parse_once(value).map_err(|error| anyhow::anyhow!(error))?,
        }));
    }
    if let Some(value) = daily {
        let time = parse_daily(value).map_err(|error| anyhow::anyhow!(error))?;
        let configured = paths.read_config()?.timezone;
        let timezone = resolve_schedule_timezone(explicit_timezone, configured.as_deref())
            .map_err(|error| anyhow::anyhow!(error))?;
        return Ok(Some(ScheduleSpec::Daily { time, timezone }));
    }
    if let Some(value) = every {
        let first_at = first_at
            .map(parse_first_at)
            .transpose()
            .map_err(|error| anyhow::anyhow!(error))?;
        if first_at.is_some_and(|value| value <= Utc::now()) {
            anyhow::bail!("--first-at must be in the future");
        }
        return Ok(Some(ScheduleSpec::Periodic {
            every: parse_every(value).map_err(|error| anyhow::anyhow!(error))?,
            first_at,
        }));
    }
    if first_at.is_some() {
        anyhow::bail!("--first-at is only valid with --every");
    }
    Ok(None)
}

pub(super) fn parse_named_dependencies(
    after: Vec<String>,
    after_failure: Vec<String>,
) -> Result<Vec<Dependency>> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for (id, status) in after
        .into_iter()
        .map(|id| (id, DependencyStatus::Succeeded))
        .chain(
            after_failure
                .into_iter()
                .map(|id| (id, DependencyStatus::Failed)),
        )
    {
        if !seen.insert(id.clone()) {
            anyhow::bail!("duplicate dependency {id:?}");
        }
        result.push(Dependency {
            upstream_task_id: id,
            status,
        });
    }
    Ok(result)
}
