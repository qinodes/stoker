//! Row mapping and transactional helpers for flow persistence.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

use crate::domain::flow::{
    AttemptState, Dependency, DependencyEvaluation, DependencyStatus, ExecutionMode,
    FlowDefinition, FlowRun, FlowRunState, FlowTask, Occurrence, OccurrenceState, SchedulePeriod,
    SchedulePeriodUnit, ScheduleSpec, TaskOutcome, TaskRun, TaskRunState, evaluate_dependencies,
};

use super::error::StoreError;

pub(super) fn require_queue_locked(connection: &Connection) -> Result<(), StoreError> {
    if connection.query_row(
        "SELECT queue_locked FROM settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )? == 0
    {
        return Err(StoreError::QueueUnlocked);
    }
    Ok(())
}

pub(super) fn require_queue_unlocked(connection: &Connection) -> Result<(), StoreError> {
    if connection.query_row(
        "SELECT queue_locked FROM settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0
    {
        return Err(StoreError::QueueLocked);
    }
    Ok(())
}

/// Keep scheduled Flow mutations safe if the workspace mode changes after the
/// HTTP adapter performed its initial guard but before this transaction starts.
pub(super) fn require_scheduled_mode(connection: &Connection) -> Result<(), StoreError> {
    let mode: String =
        connection.query_row("SELECT mode FROM settings WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    let actual = parse_mode(&mode)?;
    if actual == ExecutionMode::Scheduled {
        Ok(())
    } else {
        Err(StoreError::ScheduledModeChanged { actual })
    }
}

pub(super) fn queue_locked_with(connection: &Connection) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT queue_locked FROM settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0)
}
pub(super) fn fence_with(connection: &Connection) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT recovery_fence FROM settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

pub(super) fn has_active_work(connection: &Connection) -> Result<bool, StoreError> {
    let jobs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM jobs WHERE state IN ('STARTING','RUNNING','CANCELLING')",
        [],
        |row| row.get(0),
    )?;
    let runs: i64 = connection.query_row("SELECT COUNT(*) FROM flow_runs WHERE state IN ('STARTING','RUNNING','CANCELLING','RECOVERING')", [], |row| row.get(0))?;
    let tasks: i64 = connection.query_row(
        "SELECT COUNT(*) FROM task_runs WHERE state IN ('STARTING','RUNNING','CANCELLING')",
        [],
        |row| row.get(0),
    )?;
    let attempts: i64 = connection.query_row(
        "SELECT COUNT(*) FROM attempts WHERE state IN ('STARTING','RUNNING')",
        [],
        |row| row.get(0),
    )?;
    Ok(jobs != 0 || runs != 0 || tasks != 0 || attempts != 0 || fence_with(connection)?)
}

pub(super) fn parse_uuid(value: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value).map_err(|error| StoreError::InvalidData(error.to_string()))
}
pub(super) fn parse_mode(value: &str) -> Result<ExecutionMode, StoreError> {
    value.parse().map_err(StoreError::InvalidData)
}
pub(super) fn parse_datetime(value: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| StoreError::InvalidData(error.to_string()))
}
pub(super) fn parse_task_state(value: &str) -> Result<TaskRunState, StoreError> {
    serde_json::from_str(&format!("\"{value}\""))
        .map_err(|error| StoreError::InvalidData(error.to_string()))
}
pub(super) fn parse_flow_state(value: &str) -> Result<FlowRunState, StoreError> {
    serde_json::from_str(&format!("\"{value}\""))
        .map_err(|error| StoreError::InvalidData(error.to_string()))
}
pub(super) fn parse_dependency_status(value: &str) -> Result<DependencyStatus, String> {
    value.parse()
}

pub(super) fn task_state_string(value: TaskRunState) -> &'static str {
    match value {
        TaskRunState::Waiting => "WAITING",
        TaskRunState::Ready => "READY",
        TaskRunState::Starting => "STARTING",
        TaskRunState::Running => "RUNNING",
        TaskRunState::RetryWait => "RETRY_WAIT",
        TaskRunState::Cancelling => "CANCELLING",
        TaskRunState::Succeeded => "SUCCEEDED",
        TaskRunState::Failed => "FAILED",
        TaskRunState::Cancelled => "CANCELLED",
        TaskRunState::Skipped => "SKIPPED",
        TaskRunState::Lost => "LOST",
    }
}
pub(super) fn attempt_state_string(value: AttemptState) -> &'static str {
    match value {
        AttemptState::Starting => "STARTING",
        AttemptState::Running => "RUNNING",
        AttemptState::Succeeded => "SUCCEEDED",
        AttemptState::Failed => "FAILED",
        AttemptState::Cancelled => "CANCELLED",
        AttemptState::Lost => "LOST",
    }
}
pub(super) fn flow_state_string(value: FlowRunState) -> &'static str {
    match value {
        FlowRunState::Starting => "STARTING",
        FlowRunState::Running => "RUNNING",
        FlowRunState::Cancelling => "CANCELLING",
        FlowRunState::Recovering => "RECOVERING",
        FlowRunState::Succeeded => "SUCCEEDED",
        FlowRunState::Failed => "FAILED",
        FlowRunState::FailedToStart => "FAILED_TO_START",
        FlowRunState::Cancelled => "CANCELLED",
        FlowRunState::Skipped => "SKIPPED",
        FlowRunState::Lost => "LOST",
    }
}
pub(super) fn occurrence_state_string(value: OccurrenceState) -> &'static str {
    match value {
        OccurrenceState::Pending => "PENDING",
        OccurrenceState::Reserved => "RESERVED",
        OccurrenceState::Uncertain => "UNCERTAIN",
        OccurrenceState::Started => "STARTED",
        OccurrenceState::Replaced => "REPLACED",
        OccurrenceState::Expired => "EXPIRED",
        OccurrenceState::Superseded => "SUPERSEDED",
        OccurrenceState::Skipped => "SKIPPED",
    }
}

pub(super) fn load_flow(
    connection: &Connection,
    flow_id: &str,
) -> Result<FlowDefinition, StoreError> {
    let base = load_flow_base(connection, flow_id)?;
    let draft_json: Option<String> = connection.query_row(
        "SELECT draft_json FROM flow_definitions WHERE flow_id = ?1",
        [flow_id],
        |row| row.get(0),
    )?;
    if let Some(json) = draft_json {
        let mut draft: FlowDefinition = serde_json::from_str(&json)?;
        draft.frozen = base.frozen;
        draft.draft_revision = base.draft_revision;
        draft.has_draft = true;
        return Ok(draft);
    }
    Ok(base)
}

pub(super) fn load_flow_current(
    connection: &Connection,
    flow_id: &str,
) -> Result<FlowDefinition, StoreError> {
    let mut base = load_flow_base(connection, flow_id)?;
    // The base graph is needed by transactional commit/apply operations. A
    // draft is returned by public get/show instead.
    base.draft_revision = connection.query_row(
        "SELECT draft_revision FROM flow_definitions WHERE flow_id = ?1",
        [flow_id],
        |row| row.get(0),
    )?;
    Ok(base)
}

#[allow(clippy::type_complexity)]
pub(super) fn load_flow_base(
    connection: &Connection,
    flow_id: &str,
) -> Result<FlowDefinition, StoreError> {
    let row: Option<(String,String,String,String,String,Option<String>,Option<String>,Option<String>,Option<String>,Option<i64>,Option<String>,Option<String>,i64,i64,i64,i64,i64,i64,Option<i64>)> = connection.query_row("SELECT flow_id, internal_definition_id, name, owner, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, period_value, period_unit, period_first_at_utc, schedule_generation, committed, frozen, enabled, graph_revision, draft_revision, queue_order FROM flow_definitions WHERE flow_id = ?1", [flow_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?,row.get(13)?,row.get(14)?,row.get(15)?,row.get(16)?,row.get(17)?,row.get(18)?))).optional()?;
    let Some((
        flow_id,
        internal,
        name,
        owner,
        mode,
        kind,
        at,
        daily,
        timezone,
        period_value,
        period_unit,
        period_first_at,
        generation,
        committed,
        frozen,
        enabled,
        graph_revision,
        draft_revision,
        queue_order,
    )) = row
    else {
        return Err(StoreError::InvalidData(format!(
            "flow {flow_id:?} does not exist"
        )));
    };
    let schedule = match kind.as_deref() {
        None => None,
        Some("once") => Some(ScheduleSpec::Once {
            at: parse_datetime(&at.ok_or_else(|| {
                StoreError::InvalidData("once schedule has no timestamp".into())
            })?)?,
        }),
        Some("daily") => Some(ScheduleSpec::Daily {
            time: NaiveTime::parse_from_str(
                &daily
                    .ok_or_else(|| StoreError::InvalidData("daily schedule has no time".into()))?,
                "%H:%M",
            )
            .map_err(|error| StoreError::InvalidData(error.to_string()))?,
            timezone: timezone
                .ok_or_else(|| StoreError::InvalidData("daily schedule has no timezone".into()))?,
        }),
        Some("periodic") => Some(ScheduleSpec::Periodic {
            every: SchedulePeriod {
                value: u32::try_from(period_value.ok_or_else(|| {
                    StoreError::InvalidData("periodic schedule has no value".into())
                })?)
                .map_err(|_| StoreError::InvalidData("invalid periodic schedule value".into()))?,
                unit: match period_unit.as_deref() {
                    Some("minutes") => SchedulePeriodUnit::Minutes,
                    Some("hours") => SchedulePeriodUnit::Hours,
                    _ => {
                        return Err(StoreError::InvalidData(
                            "periodic schedule has an invalid unit".into(),
                        ));
                    }
                },
            },
            first_at: period_first_at.as_deref().map(parse_datetime).transpose()?,
        }),
        Some(other) => {
            return Err(StoreError::InvalidData(format!(
                "unknown schedule kind {other}"
            )));
        }
    };
    let task_rows = connection.prepare("SELECT task_id, name, cwd, command_line, retry, depend_mode, sequence FROM flow_tasks WHERE flow_id = ?1 ORDER BY sequence")?.query_map([flow_id.as_str()], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, i64>(4)?,row.get::<_, String>(5)?,row.get::<_, i64>(6)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut tasks = Vec::new();
    for (task_id, task_name, cwd, command, retry, depend_mode, sequence) in task_rows {
        let dependencies = connection.prepare("SELECT upstream_task_id, status FROM flow_dependencies WHERE flow_id = ?1 AND task_id = ?2 ORDER BY upstream_task_id, status")?.query_map(params![flow_id.as_str(), task_id.as_str()], |row| Ok(Dependency { upstream_task_id: row.get(0)?, status: parse_dependency_status(&row.get::<_, String>(1)?).map_err(|_| rusqlite::Error::InvalidQuery)? }))?.collect::<Result<Vec<_>, _>>()?;
        tasks.push(FlowTask {
            task_id,
            name: task_name,
            cwd,
            command,
            retry: u32::try_from(retry)
                .map_err(|_| StoreError::InvalidData("invalid retry".into()))?,
            dependencies,
            depend_mode: depend_mode.parse().map_err(StoreError::InvalidData)?,
            sequence: u64::try_from(sequence)
                .map_err(|_| StoreError::InvalidData("invalid task sequence".into()))?,
        });
    }
    Ok(FlowDefinition {
        flow_id,
        internal_id: parse_uuid(&internal)?,
        name,
        owner,
        mode: parse_mode(&mode)?,
        schedule,
        tasks,
        committed: committed != 0,
        frozen: frozen != 0,
        enabled: enabled != 0,
        graph_revision,
        schedule_generation: generation,
        draft_revision,
        has_draft: false,
        queue_order,
    })
}

pub(super) fn insert_task(
    transaction: &Transaction<'_>,
    flow_id: &str,
    task: &FlowTask,
) -> Result<(), StoreError> {
    transaction.execute("INSERT INTO flow_tasks (flow_id, task_id, name, cwd, command, command_line, retry, depend_mode, sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", params![flow_id, task.task_id, task.name, task.cwd, serde_json::to_string(std::slice::from_ref(&task.command))?, task.command, i64::from(task.retry), task.depend_mode.to_string(), i64::try_from(task.sequence).map_err(|_| StoreError::InvalidData("task sequence overflow".into()))?])?;
    for edge in &task.dependencies {
        transaction.execute("INSERT INTO flow_dependencies (flow_id, task_id, upstream_task_id, status) VALUES (?1, ?2, ?3, ?4)", params![flow_id, task.task_id, edge.upstream_task_id, edge.status.to_string()])?;
    }
    Ok(())
}

pub(super) fn replace_tasks(
    transaction: &Transaction<'_>,
    definition: &FlowDefinition,
) -> Result<(), StoreError> {
    transaction.execute(
        "DELETE FROM flow_tasks WHERE flow_id = ?1",
        [&definition.flow_id],
    )?;
    for task in &definition.tasks {
        insert_task(transaction, &definition.flow_id, task)?;
    }
    Ok(())
}

pub(super) fn save_draft(
    transaction: &Transaction<'_>,
    definition: &FlowDefinition,
    revision: i64,
) -> Result<(), StoreError> {
    let mut draft = definition.clone();
    draft.draft_revision = revision;
    draft.frozen = true;
    transaction.execute(
        "UPDATE flow_definitions SET draft_json = ?2, draft_revision = ?3 WHERE flow_id = ?1",
        params![definition.flow_id, serde_json::to_string(&draft)?, revision],
    )?;
    Ok(())
}

pub(super) fn load_run(connection: &Connection, run_id: Uuid) -> Result<FlowRun, StoreError> {
    let (flow_id, generation, source, state, occurrence, started, finished): (String,i64,String,String,Option<String>,Option<String>,Option<String>) = connection.query_row("SELECT flow_id, schedule_generation, source, state, occurrence_id, started_at, finished_at FROM flow_runs WHERE run_id = ?1", [run_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)))?;
    Ok(FlowRun {
        run_id,
        flow_id,
        generation,
        source,
        state: parse_flow_state(&state)?,
        occurrence_id: occurrence.map(|value| parse_uuid(&value)).transpose()?,
        started_at: started.map(|value| parse_datetime(&value)).transpose()?,
        finished_at: finished.map(|value| parse_datetime(&value)).transpose()?,
        tasks: query_task_runs(connection, run_id)?,
    })
}

pub(super) fn query_task_runs(
    connection: &Connection,
    run_id: Uuid,
) -> Result<Vec<TaskRun>, StoreError> {
    Ok(connection.prepare("SELECT task_id, state, attempt_count, next_attempt_at, cancel_requested FROM task_runs WHERE run_id = ?1 ORDER BY task_id")?.query_map([run_id.to_string()], |row| {
        let next: Option<String> = row.get(3)?;
        Ok(TaskRun { run_id, task_id: row.get(0)?, state: parse_task_state(&row.get::<_, String>(1)?).map_err(|error| to_sql_error(error.to_string()))?, attempt_count: u32::try_from(row.get::<_, i64>(2)?).map_err(|_| rusqlite::Error::InvalidQuery)?, next_attempt_at: next.map(|value| parse_datetime(&value).map_err(|error| to_sql_error(error.to_string()))).transpose()?, cancel_requested: row.get::<_, i64>(4)? != 0 })
    })?.collect::<Result<Vec<_>, _>>()?)
}

pub(super) fn occurrence_from_row(row: &Row<'_>) -> rusqlite::Result<Occurrence> {
    let id: String = row.get(0)?;
    Ok(Occurrence {
        occurrence_id: Uuid::parse_str(&id).map_err(|_| rusqlite::Error::InvalidQuery)?,
        flow_id: row.get(1)?,
        generation: row.get(2)?,
        local_date: row
            .get::<_, Option<String>>(3)?
            .map(|value| {
                NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .transpose()?,
        due_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .with_timezone(&Utc),
        state: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(5)?))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        reason: row.get(6)?,
    })
}

pub(super) fn to_sql_error(error: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
    )
}

pub(super) fn task_outcomes(tasks: &[TaskRun]) -> BTreeMap<String, TaskOutcome> {
    tasks
        .iter()
        .filter_map(|task| {
            let outcome = match task.state {
                TaskRunState::Succeeded => TaskOutcome::Succeeded,
                TaskRunState::Failed => TaskOutcome::Failed,
                TaskRunState::Cancelled => TaskOutcome::Cancelled,
                TaskRunState::Skipped => TaskOutcome::Skipped,
                TaskRunState::Lost => TaskOutcome::Lost,
                _ => return None,
            };
            Some((task.task_id.clone(), outcome))
        })
        .collect()
}

pub(super) fn propagate_task_states(
    transaction: &Transaction<'_>,
    run_id: Uuid,
) -> Result<(), StoreError> {
    let snapshot: String = transaction.query_row(
        "SELECT definition_snapshot FROM flow_runs WHERE run_id = ?1",
        [run_id.to_string()],
        |row| row.get(0),
    )?;
    let definition: FlowDefinition = serde_json::from_str(&snapshot)?;
    for _ in 0..definition.tasks.len().max(1) {
        let current = query_task_runs(transaction, run_id)?;
        let outcomes = task_outcomes(&current);
        let mut changed = false;
        for task in &definition.tasks {
            let Some(task_run) = current.iter().find(|run| run.task_id == task.task_id) else {
                continue;
            };
            if task_run.state != TaskRunState::Waiting {
                continue;
            }
            match evaluate_dependencies(task, &outcomes) {
                DependencyEvaluation::Ready => {
                    transaction.execute(
                        "UPDATE task_runs SET state = 'READY' WHERE run_id = ?1 AND task_id = ?2",
                        params![run_id.to_string(), task.task_id],
                    )?;
                    changed = true;
                }
                DependencyEvaluation::Skipped => {
                    transaction.execute(
                        "UPDATE task_runs SET state = 'SKIPPED' WHERE run_id = ?1 AND task_id = ?2",
                        params![run_id.to_string(), task.task_id],
                    )?;
                    changed = true;
                }
                DependencyEvaluation::Waiting => {}
            }
        }
        if !changed {
            break;
        }
    }
    Ok(())
}
