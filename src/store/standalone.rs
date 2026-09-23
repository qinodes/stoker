//! Standalone job scheduling metadata stored through the flow runtime.

use chrono::{NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::flow::{
    DependencyMode, ExecutionMode, FlowRun, FlowTask, OccurrenceState, SchedulePeriod,
    SchedulePeriodUnit, ScheduleSpec, validate_definition,
};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{insert_task, load_flow_base, parse_datetime, parse_mode, parse_uuid};
use super::flow_runtime_mapping::{insert_occurrence, schedule_from_row};
use super::flow_schedule_mapping::{
    activate_periodic_schedule, schedule_columns, validate_schedule,
};
use super::flows::StandaloneDefinition;
use super::mapping::get_job_with;

pub(super) fn standalone_flow_id(job_id: Uuid) -> String {
    format!("standalone/{job_id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualRequestStatus {
    pub request_id: Uuid,
    pub flow_id: String,
    pub run_id: Option<Uuid>,
    pub result: String,
}

fn standalone_definition_from(
    connection: &Connection,
    job_id: Uuid,
) -> Result<StandaloneDefinition, StoreError> {
    connection.query_row("SELECT mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, period_value, period_unit, period_first_at_utc, retry, enabled, schedule_generation FROM jobs WHERE id = ?1", [job_id.to_string()], |row| Ok(StandaloneDefinition { job_id, mode: parse_mode(&row.get::<_, String>(0)?).map_err(|error| super::flow_mapping::to_sql_error(error.to_string()))?, schedule: schedule_from_row(row, 1).map_err(super::flow_mapping::to_sql_error)?, retry: u32::try_from(row.get::<_, i64>(8)?).map_err(|_| rusqlite::Error::InvalidQuery)?, enabled: row.get::<_, i64>(9)? != 0, generation: row.get(10)? })).optional()?.ok_or(StoreError::NotFound { id: job_id })
}

impl Store {
    pub fn manual_request(&self, request_id: Uuid) -> Result<ManualRequestStatus, StoreError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT flow_id, run_id, result FROM manual_requests WHERE request_id = ?1",
                [request_id.to_string()],
                |row| {
                    Ok(ManualRequestStatus {
                        request_id,
                        flow_id: row.get(0)?,
                        run_id: row
                            .get::<_, Option<String>>(1)?
                            .map(|value| {
                                parse_uuid(&value).map_err(|error| {
                                    super::flow_mapping::to_sql_error(error.to_string())
                                })
                            })
                            .transpose()?,
                        result: row.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or(StoreError::InvalidData(format!(
                "request {request_id} does not exist"
            )))
    }

    /// Create a manual run against the standalone job's stable hidden flow.
    /// The original job definition and its future schedule remain untouched.
    pub fn create_standalone_run(
        &self,
        job_id: Uuid,
        skip_next: bool,
        request_id: Option<Uuid>,
    ) -> Result<FlowRun, StoreError> {
        let flow_id = standalone_flow_id(job_id);
        let connection = self.lock()?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM flow_definitions WHERE flow_id = ?1 AND committed = 1)",
            [&flow_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::InvalidData(
                "scheduled standalone job has no committed durable definition".into(),
            ));
        }
        drop(connection);
        self.create_flow_run(&flow_id, "MANUAL", skip_next, request_id)
    }

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
        let current_mode: String =
            transaction.query_row("SELECT mode FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        let current_mode = parse_mode(&current_mode)?;
        if current_mode != mode {
            return Err(StoreError::ScheduledModeChanged {
                actual: current_mode,
            });
        }
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
        let columns = schedule_columns(schedule.as_ref());
        transaction.execute("UPDATE jobs SET mode = ?2, schedule_kind = ?3, schedule_at_utc = ?4, daily_time = ?5, schedule_timezone = ?6, period_value = ?7, period_unit = ?8, period_first_at_utc = ?9, retry = ?10 WHERE id = ?1", params![job_id.to_string(), mode.to_string(), columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, i64::from(retry)])?;
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
    let (name, owner, cwd, command_line, command_json, retry, mode, kind, at, daily, timezone, period_value, period_unit, period_first_at, generation): (
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
        Option<i64>,
        Option<String>,
        Option<String>,
        i64,
    ) = transaction.query_row(
        "SELECT name, user, cwd, command_line, command, retry, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, period_value, period_unit, period_first_at_utc, schedule_generation FROM jobs WHERE id = ?1",
        [job_id.to_string()],
        |row| Ok((
            row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
            row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?,
            row.get(12)?, row.get(13)?, row.get(14)?,
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
    let columns = schedule_columns(schedule.as_ref());
    if exists {
        transaction.execute(
            "UPDATE flow_definitions SET name = ?2, owner = ?3, mode = ?4, schedule_kind = ?5, schedule_at_utc = ?6, daily_time = ?7, schedule_timezone = ?8, period_value = ?9, period_unit = ?10, period_first_at_utc = ?11, schedule_generation = ?12, committed = 0, frozen = 0, enabled = 1, graph_revision = 0, draft_revision = 0, draft_json = NULL, queue_order = NULL, committed_at = NULL WHERE flow_id = ?1",
            params![flow_id, name, owner, mode.to_string(), columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, generation],
        )?;
        transaction.execute("DELETE FROM occurrences WHERE flow_id = ?1", [&flow_id])?;
        transaction.execute("DELETE FROM flow_tasks WHERE flow_id = ?1", [&flow_id])?;
    } else {
        transaction.execute(
            "INSERT INTO flow_definitions (flow_id, internal_definition_id, name, owner, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, period_value, period_unit, period_first_at_utc, schedule_generation, committed, frozen, enabled, graph_revision, draft_revision, created_at, last_dispatch_sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 0, 1, 0, 0, ?14, 0)",
            params![flow_id, job_id.to_string(), name, owner, mode.to_string(), columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, generation, now],
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
    let mut definition = load_flow_base(connection, &flow_id)?;
    let commit_instant = parse_datetime(committed_at)?;
    activate_periodic_schedule(&mut definition.schedule, commit_instant)?;
    validate_definition(&definition).map_err(StoreError::InvalidData)?;
    let columns = schedule_columns(definition.schedule.as_ref());
    connection.execute(
        "UPDATE flow_definitions SET committed = 1, queue_order = ?2, committed_at = ?3, graph_revision = 1, frozen = 0, schedule_kind = ?4, schedule_at_utc = ?5, daily_time = ?6, schedule_timezone = ?7, period_value = ?8, period_unit = ?9, period_first_at_utc = ?10 WHERE flow_id = ?1",
        params![flow_id, queue_order, committed_at, columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at],
    )?;
    connection.execute(
        "UPDATE jobs SET schedule_kind = ?2, schedule_at_utc = ?3, daily_time = ?4, schedule_timezone = ?5, period_value = ?6, period_unit = ?7, period_first_at_utc = ?8 WHERE id = ?1",
        params![job_id.to_string(), columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at],
    )?;
    let first_occurrence = match definition.schedule.as_ref() {
        Some(ScheduleSpec::Once { at }) => Some(*at),
        Some(ScheduleSpec::Periodic { first_at, .. }) => *first_at,
        _ => None,
    };
    if let Some(at) = first_occurrence {
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
    let columns = schedule_columns(definition.schedule.as_ref());
    let next_order: i64 = connection.query_row(
        "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM jobs WHERE state = 'QUEUED'",
        [],
        |row| row.get(0),
    )?;
    connection.execute(
        "UPDATE jobs SET mode = ?2, schedule_kind = ?3, schedule_at_utc = ?4, daily_time = ?5, schedule_timezone = ?6, period_value = ?7, period_unit = ?8, period_first_at_utc = ?9, schedule_generation = ?10, retry = ?11, enabled = ?12, queue_order = CASE WHEN ?13 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?12 = 1 THEN ?14 ELSE queue_order END, state = CASE WHEN ?13 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?12 = 1 THEN 'QUEUED' ELSE state END, started_at = CASE WHEN ?13 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?12 = 1 THEN NULL ELSE started_at END, finished_at = CASE WHEN ?13 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?12 = 1 THEN NULL ELSE finished_at END, exit_code = CASE WHEN ?13 = 1 AND state NOT IN ('STARTING','RUNNING','CANCELLING') AND ?12 = 1 THEN NULL ELSE exit_code END WHERE id = ?1",
        params![
            job_id.to_string(),
            definition.mode.to_string(),
            columns.kind,
            columns.at,
            columns.daily,
            columns.timezone,
            columns.period_value,
            columns.period_unit,
            columns.period_first_at,
            definition.schedule_generation,
            i64::from(task.retry),
            i64::from(definition.enabled),
            i64::from(requeue),
            next_order,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewJob;

    fn draft_job(store: &Store, root: &std::path::Path) -> Uuid {
        store
            .create_job(NewJob {
                name: "standalone fixture".into(),
                user: "tester".into(),
                description: None,
                cwd: root.to_path_buf(),
                command: vec!["echo".into(), "fixture".into()],
            })
            .unwrap()
    }

    #[test]
    fn repeated_draft_configuration_replaces_the_hidden_definition() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("stoker.db")).unwrap();
        let id = draft_job(&store, directory.path());
        assert!(
            matches!(store.create_standalone_run(id, false, None), Err(StoreError::InvalidData(message)) if message.contains("no committed"))
        );
        let first = store
            .configure_standalone(id, ExecutionMode::Serial, None, 1)
            .unwrap();
        assert_eq!(first.retry, 1);
        let second = store
            .configure_standalone(id, ExecutionMode::Serial, None, 3)
            .unwrap();
        assert_eq!(second.retry, 3);
        let hidden = store.get_flow(&standalone_flow_id(id)).unwrap();
        assert_eq!(hidden.tasks.len(), 1);
        assert_eq!(hidden.tasks[0].retry, 3);
        assert!(!hidden.committed);
        store.commit_job(id).unwrap();
        assert!(
            matches!(store.configure_standalone(id, ExecutionMode::Serial, None, 0), Err(StoreError::InvalidData(message)) if message.contains("DRAFT"))
        );
        assert!(
            matches!(store.manual_request(Uuid::new_v4()), Err(StoreError::InvalidData(message)) if message.contains("does not exist"))
        );
    }

    #[test]
    fn hidden_definition_sync_rejects_malformed_persisted_schedule_data() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("stoker.db")).unwrap();
        let id = draft_job(&store, directory.path());
        let mut connection = store.lock().unwrap();
        for (kind, columns, message) in [
            ("once", "schedule_at_utc = NULL", "timestamp"),
            (
                "daily",
                "daily_time = NULL, schedule_timezone = 'UTC'",
                "time",
            ),
            (
                "daily",
                "daily_time = '08:30', schedule_timezone = NULL",
                "timezone",
            ),
            (
                "periodic",
                "period_value = NULL, period_unit = 'minutes'",
                "value",
            ),
            ("periodic", "period_value = 1, period_unit = 'days'", "unit"),
            ("unknown", "schedule_at_utc = NULL", "unknown schedule kind"),
        ] {
            let transaction = connection.transaction().unwrap();
            transaction.execute("UPDATE jobs SET mode = 'scheduled', schedule_kind = ?2, schedule_at_utc = NULL, daily_time = NULL, schedule_timezone = NULL, period_value = NULL, period_unit = NULL WHERE id = ?1", params![id.to_string(), kind]).unwrap();
            transaction
                .execute(
                    &format!("UPDATE jobs SET {columns} WHERE id = ?1"),
                    [id.to_string()],
                )
                .unwrap();
            let error = sync_standalone_flow(&transaction, id).unwrap_err();
            assert!(error.to_string().contains(message), "{kind}: {error}");
        }
    }
}
