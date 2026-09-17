//! Input validation and schedule parsing for the extended CLI.

use std::collections::BTreeSet;

use anyhow::Result;

use crate::StokerPaths;
use crate::domain::flow::{
    Dependency, DependencyStatus, ScheduleSpec, parse_daily, parse_once, resolve_schedule_timezone,
};

pub(super) fn parse_schedule(
    paths: &StokerPaths,
    at: Option<&str>,
    daily: Option<&str>,
    explicit_timezone: Option<&str>,
) -> Result<Option<ScheduleSpec>> {
    match (at, daily) {
        (Some(at), None) => Ok(Some(ScheduleSpec::Once {
            at: parse_once(at).map_err(|error| anyhow::anyhow!(error))?,
        })),
        (None, Some(daily)) => {
            let time = parse_daily(daily).map_err(|error| anyhow::anyhow!(error))?;
            let configured = paths.read_config()?.timezone;
            let timezone = resolve_schedule_timezone(explicit_timezone, configured.as_deref())
                .map_err(|error| anyhow::anyhow!(error))?;
            Ok(Some(ScheduleSpec::Daily { time, timezone }))
        }
        (None, None) => Ok(None),
        (Some(_), Some(_)) => anyhow::bail!("--at and --daily are mutually exclusive"),
    }
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
