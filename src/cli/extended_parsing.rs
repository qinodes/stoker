//! Input validation and schedule parsing for the extended CLI.

use anyhow::{Context, Result};

use crate::StokerPaths;
use crate::domain::flow::{
    Dependency, DependencyMode, ScheduleSpec, parse_daily, parse_once, resolve_schedule_timezone,
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

pub(super) fn parse_dependencies(
    single_id: Option<String>,
    single_status: Option<String>,
    multiple: Vec<String>,
    mode: String,
) -> Result<Vec<Dependency>> {
    if single_id.is_some() != single_status.is_some() {
        anyhow::bail!("--depend-on and --depend-status must be provided together");
    }
    if single_id.is_some() && !multiple.is_empty() {
        anyhow::bail!("single dependency flags cannot be mixed with --dependency");
    }
    let mut result = Vec::new();
    if let (Some(id), Some(status)) = (single_id, single_status) {
        result.push(Dependency {
            upstream_task_id: id,
            status: status
                .parse()
                .map_err(|error: String| anyhow::anyhow!(error))?,
        });
    }
    result.extend(parse_dependency_values(multiple)?);
    let _ = parse_depend_mode(&mode)?;
    Ok(result)
}
pub(super) fn parse_dependency_values(values: Vec<String>) -> Result<Vec<Dependency>> {
    let mut result = Vec::new();
    for value in values {
        let (id, status) = value
            .split_once(':')
            .context("--dependency must use TASK_ID:succeeded|failed")?;
        if result
            .iter()
            .any(|edge: &Dependency| edge.upstream_task_id == id)
        {
            anyhow::bail!("duplicate dependency {id:?}");
        }
        result.push(Dependency {
            upstream_task_id: id.to_owned(),
            status: status
                .parse()
                .map_err(|error: String| anyhow::anyhow!(error))?,
        });
    }
    Ok(result)
}
pub(super) fn parse_depend_mode(value: &str) -> Result<DependencyMode> {
    value
        .parse()
        .map_err(|error: String| anyhow::anyhow!(error))
}
