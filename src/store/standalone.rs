//! Standalone job scheduling metadata owned by the Task 003 store.

use chrono::{NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::flow::{
    DependencyMode, ExecutionMode, FlowTask, OccurrenceState, ScheduleSpec, validate_definition,
};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{
    insert_task, load_flow_base, parse_datetime, parse_mode, parse_uuid, schedule_columns,
    validate_schedule,
};
use super::flow_runtime_mapping::{insert_occurrence, schedule_from_row};
use super::flows::StandaloneDefinition;
use super::mapping::get_job_with;

pub(super) fn standalone_flow_id(job_id: Uuid) -> String {
    format!("standalone/{job_id}")
}

fn standalone_definition_from(
    connection: &Connection,
    job_id: Uuid,
) -> Result<StandaloneDefinition, StoreError> {
    connection.query_row("SELECT mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, retry, enabled, schedule_generation FROM jobs WHERE id = ?1", [job_id.to_string()], |row| Ok(StandaloneDefinition { job_id, mode: parse_mode(&row.get::<_, String>(0)?).map_err(|error| super::flow_mapping::to_sql_error(error.to_string()))?, schedule: schedule_from_row(row, 1).map_err(super::flow_mapping::to_sql_error)?, retry: u32::try_from(row.get::<_, i64>(5)?).map_err(|_| rusqlite::Error::InvalidQuery)?, enabled: row.get::<_, i64>(6)? != 0, generation: row.get(7)? })).optional()?.ok_or(StoreError::NotFound { id: job_id })
}

impl Store {
    pub fn standalone_definition(&self, job_id: Uuid) -> Result<StandaloneDefinition, StoreError> {
        let connection = self.lock()?;
        standalone_definition_from(&connection, job_id)
    }

    pub fn configure_standalone(
        &self,
        job_id: Uuid,
        mode: ExecutionMode,
        schedule: Option<ScheduleSpec>,
        retry: u32,
    ) -> Result<StandaloneDefinition, StoreError> {
        validate_schedule(mode, schedule.as_ref())?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: String = transaction.query_row(
            "SELECT state FROM jobs WHERE id = ?1",
            [job_id.to_string()],
            |row| row.get(0),
        )?;
        if state != "DRAFT" {
            return Err(StoreError::InvalidData(
                "schedule options can only be set on a DRAFT job".into(),
            ));
        }
        let (kind, at, daily, timezone) = schedule_columns(schedule.as_ref());
        transaction.execute("UPDATE jobs SET mode = ?2, schedule_kind = ?3, schedule_at_utc = ?4, daily_time = ?5, schedule_timezone = ?6, retry = ?7 WHERE id = ?1", params![job_id.to_string(), mode.to_string(), kind, at, daily, timezone, i64::from(retry)])?;
        sync_standalone_flow(&transaction, job_id)?;
        let definition = standalone_definition_from(&transaction, job_id)?;
        transaction.commit()?;
        Ok(definition)
    }

    pub fn list_jobs_for_mode(
        &self,
        owner: Option<&str>,
        state: Option<crate::JobState>,
        mode: Option<ExecutionMode>,
    ) -> Result<Vec<crate::Job>, StoreError> {
        let connection = self.lock()?;
        let state = state.map(|value| value.to_string());
        let mode = mode.map(|value| value.to_string());
        let ids = connection.prepare("SELECT id FROM jobs WHERE (?1 IS NULL OR user = ?1) AND (?2 IS NULL OR state = ?2) AND (?3 IS NULL OR mode = ?3) ORDER BY CASE WHEN state = 'QUEUED' THEN 0 ELSE 1 END, queue_order, created_at DESC, id DESC")?.query_map(params![owner, state, mode], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| get_job_with(&connection, parse_uuid(&id)?))
            .collect()
    }
}

/// Keep the durable flow representation of a standalone job in sync while it
/// is still a draft.  The flow is intentionally hidden from `flow list`, but
/// its stable identity gives scheduled jobs the same durable run model as
/// user-created flows.
#[allow(clippy::type_complexity)]
fn sync_standalone_flow(transaction: &Transaction<'_>, job_id: Uuid) -> Result<(), StoreError> {
    let (name, owner, cwd, command_line, command_json, retry, mode, kind, at, daily, timezone, generation): (
        String,
        String,
        String,
        Option<String>,
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    ) = transaction.query_row(
        "SELECT name, user, cwd, command_line, command, retry, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, schedule_generation FROM jobs WHERE id = ?1",
        [job_id.to_string()],
        |row| Ok((
            row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
            row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?,
        )),
    )?;
    let mode = parse_mode(&mode)?;
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
        Some(other) => {
            return Err(StoreError::InvalidData(format!(
                "unknown schedule kind {other}"
            )));
        }
    };
    validate_schedule(mode, schedule.as_ref())?;
    let command = command_line.unwrap_or_else(|| {
        serde_json::from_str::<Vec<String>>(&command_json)
            .unwrap_or_default()
            .join(" ")
    });
    let flow_id = standalone_flow_id(job_id);
    let task = FlowTask {
        task_id: "job".into(),
        name: name.clone(),
        cwd,
        command,
        retry: u32::try_from(retry).map_err(|_| StoreError::InvalidData("invalid retry".into()))?,
        dependencies: Vec::new(),
        depend_mode: DependencyMode::All,
        sequence: 0,
    };
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM flow_definitions WHERE flow_id = ?1)",
        [&flow_id],
        |row| row.get(0),
    )?;
    let now = Utc::now().to_rfc3339();
    let (schedule_kind, schedule_at, daily_time, schedule_timezone) =
        schedule_columns(schedule.as_ref());
    if exists {
        transaction.execute(
            "UPDATE flow_definitions SET name = ?2, owner = ?3, mode = ?4, schedule_kind = ?5, schedule_at_utc = ?6, daily_time = ?7, schedule_timezone = ?8, schedule_generation = ?9, committed = 0, frozen = 0, enabled = 1, graph_revision = 0, draft_revision = 0, draft_json = NULL, queue_order = NULL, committed_at = NULL WHERE flow_id = ?1",
            params![flow_id, name, owner, mode.to_string(), schedule_kind, schedule_at, daily_time, schedule_timezone, generation],
        )?;
        transaction.execute("DELETE FROM occurrences WHERE flow_id = ?1", [&flow_id])?;
        transaction.execute("DELETE FROM flow_tasks WHERE flow_id = ?1", [&flow_id])?;
    } else {
        transaction.execute(
            "INSERT INTO flow_definitions (flow_id, internal_definition_id, name, owner, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, schedule_generation, committed, frozen, enabled, graph_revision, draft_revision, created_at, last_dispatch_sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0, 1, 0, 0, ?11, 0)",
            params![flow_id, job_id.to_string(), name, owner, mode.to_string(), schedule_kind, schedule_at, daily_time, schedule_timezone, generation, now],
        )?;
    }
    insert_task(transaction, &flow_id, &task)?;
    Ok(())
}

/// Promote a scheduled standalone job's hidden definition at the same time
/// its legacy job row is committed.
pub(super) fn commit_standalone_definition(
    connection: &Connection,
    job_id: Uuid,
    queue_order: i64,
    committed_at: &str,
) -> Result<(), StoreError> {
    let flow_id = standalone_flow_id(job_id);
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM flow_definitions WHERE flow_id = ?1)",
        [&flow_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    let definition = load_flow_base(connection, &flow_id)?;
    validate_definition(&definition).map_err(StoreError::InvalidData)?;
    connection.execute(
        "UPDATE flow_definitions SET committed = 1, queue_order = ?2, committed_at = ?3, graph_revision = 1, frozen = 0 WHERE flow_id = ?1",
        params![flow_id, queue_order, committed_at],
    )?;
    if let Some(ScheduleSpec::Once { at }) = definition.schedule {
        insert_occurrence(
            connection,
            &definition,
            None,
            at,
            OccurrenceState::Pending,
            None,
        )?;
    }
    Ok(())
}

pub(super) fn sync_standalone_job_from_flow(
    connection: &Connection,
    definition: &crate::domain::flow::FlowDefinition,
    requeue: bool,
) -> Result<(), StoreError> {
    let Some(job_text) = definition.flow_id.strip_prefix("standalone/") else {
        return Ok(());
    };
    if job_text.contains('/') {
        return Ok(());
    }
    let job_id = parse_uuid(job_text)?;
    let Some(task) = definition.tasks.first() else {
        return Ok(());
    };
    let (kind, at, daily, timezone) = schedule_columns(definition.schedule.as_ref());
    let next_order: i64 = connection.query_row(
        "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM jobs WHERE state = 'QUEUED'",
        [],
        |row| row.get(0),
    )?;
    connection.execute(
        "UPDATE jobs SET mode = ?2, schedule_kind = ?3, schedule_at_utc = ?4, daily_time = ?5, schedule_timezone = ?6, schedule_generation = ?7, retry = ?8, enabled = ?9, queue_order = CASE WHEN ?10 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?9 = 1 THEN ?11 ELSE queue_order END, state = CASE WHEN ?10 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?9 = 1 THEN 'QUEUED' ELSE state END, started_at = CASE WHEN ?10 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?9 = 1 THEN NULL ELSE started_at END, finished_at = CASE WHEN ?10 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?9 = 1 THEN NULL ELSE finished_at END, exit_code = CASE WHEN ?10 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?9 = 1 THEN NULL ELSE exit_code END WHERE id = ?1",
        params![
            job_id.to_string(),
            definition.mode.to_string(),
            kind,
            at,
            daily,
            timezone,
            definition.schedule_generation,
            i64::from(task.retry),
            i64::from(definition.enabled),
            i64::from(requeue),
            next_order,
        ],
    )?;
    Ok(())
}
