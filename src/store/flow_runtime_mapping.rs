//! Runtime and schedule row helpers for flow persistence.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

use crate::domain::flow::{
    FlowDefinition, OccurrenceState, SchedulePeriod, SchedulePeriodUnit, ScheduleSpec,
};

use super::error::StoreError;
use super::flow_mapping::*;

fn standalone_job_id(flow_id: &str) -> Option<Uuid> {
    let value = flow_id.strip_prefix("standalone/")?;
    if value.contains('/') {
        return None;
    }
    Uuid::parse_str(value).ok()
}

pub(super) fn sync_standalone_job_start(
    connection: &Connection,
    flow_id: &str,
) -> Result<(), StoreError> {
    let Some(job_id) = standalone_job_id(flow_id) else {
        return Ok(());
    };
    connection.execute(
        "UPDATE jobs SET state = 'STARTING', queue_order = NULL, started_at = ?2 WHERE id = ?1 AND state IN ('QUEUED', 'STARTING')",
        params![job_id.to_string(), Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(super) fn sync_standalone_job_running(
    connection: &Connection,
    flow_id: &str,
) -> Result<(), StoreError> {
    let Some(job_id) = standalone_job_id(flow_id) else {
        return Ok(());
    };
    connection.execute(
        "UPDATE jobs SET state = 'RUNNING' WHERE id = ?1 AND state IN ('QUEUED', 'STARTING')",
        [job_id.to_string()],
    )?;
    Ok(())
}

pub(super) fn sync_standalone_job_terminal(
    connection: &Connection,
    flow_id: &str,
    state: &str,
) -> Result<(), StoreError> {
    let Some(job_id) = standalone_job_id(flow_id) else {
        return Ok(());
    };
    let schedule_kind: Option<String> = connection.query_row(
        "SELECT schedule_kind FROM jobs WHERE id = ?1",
        [job_id.to_string()],
        |row| row.get(0),
    )?;
    if schedule_kind.as_deref() == Some("daily") {
        let next_order: i64 = connection.query_row(
            "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM jobs WHERE state = 'QUEUED'",
            [],
            |row| row.get(0),
        )?;
        connection.execute(
            "UPDATE jobs SET state = 'QUEUED', queue_order = ?2, started_at = NULL, finished_at = NULL, exit_code = NULL, failure_detail = NULL WHERE id = ?1",
            params![job_id.to_string(), next_order],
        )?;
        return Ok(());
    }
    let job_state = match state {
        "SUCCEEDED" => "SUCCEEDED",
        "CANCELLED" => "CANCELLED",
        _ => "FAILED",
    };
    connection.execute(
        "UPDATE jobs SET state = ?2, queue_order = NULL, finished_at = ?3 WHERE id = ?1",
        params![job_id.to_string(), job_state, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(super) fn ensure_run_owner(
    transaction: &Transaction<'_>,
    flow_id: &str,
    run_id: Uuid,
) -> Result<(), StoreError> {
    let owner: String = transaction.query_row(
        "SELECT flow_id FROM flow_runs WHERE run_id = ?1",
        [run_id.to_string()],
        |row| row.get(0),
    )?;
    if owner != flow_id {
        return Err(StoreError::InvalidData(
            "run does not belong to flow".into(),
        ));
    }
    Ok(())
}

pub(super) fn reserve_next_occurrence(
    transaction: &Transaction<'_>,
    definition: &FlowDefinition,
) -> Result<Uuid, StoreError> {
    let now = Utc::now();
    let row: Option<String> = transaction.query_row("SELECT occurrence_id FROM occurrences WHERE flow_id = ?1 AND schedule_generation = ?2 AND state = 'PENDING' AND ((kind IN ('daily','periodic') AND due_at > ?3) OR (kind = 'once' AND due_at > ?4)) ORDER BY due_at LIMIT 1", params![definition.flow_id, definition.schedule_generation, now.to_rfc3339(), (now - chrono::Duration::hours(24)).to_rfc3339()], |row| row.get(0)).optional()?;
    let Some(value) = row else {
        return Err(StoreError::InvalidData(
            "no future occurrence is available to replace".into(),
        ));
    };
    transaction.execute(
        "UPDATE occurrences SET state = 'RESERVED' WHERE occurrence_id = ?1 AND state = 'PENDING'",
        [&value],
    )?;
    parse_uuid(&value)
}

pub(super) fn ensure_next_daily_occurrence(
    transaction: &Transaction<'_>,
    definition: &FlowDefinition,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let Some(schedule) = definition.schedule.as_ref() else {
        return Ok(());
    };
    let next = schedule
        .next_daily_after(now)
        .map_err(StoreError::InvalidData)?;
    if !occurrence_exists(
        transaction,
        &definition.internal_id.to_string(),
        definition.schedule_generation,
        "daily",
        &next.local_date.to_string(),
    )? {
        insert_occurrence(
            transaction,
            definition,
            Some(next.local_date),
            next.due_at,
            OccurrenceState::Pending,
            None,
        )?;
    }
    Ok(())
}

pub(super) fn ensure_next_periodic_occurrence(
    transaction: &Transaction<'_>,
    definition: &FlowDefinition,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let Some(schedule @ ScheduleSpec::Periodic { .. }) = definition.schedule.as_ref() else {
        return Ok(());
    };
    let (_, next) = schedule
        .periodic_window(now)
        .map_err(StoreError::InvalidData)?;
    insert_periodic_occurrence_if_missing(
        transaction,
        definition,
        next,
        OccurrenceState::Pending,
        None,
    )?;
    Ok(())
}

pub(super) fn next_definition_order(transaction: &Transaction<'_>) -> Result<i64, StoreError> {
    Ok(transaction.query_row("SELECT COALESCE(MAX(value), 0) + 1 FROM (SELECT COALESCE(MAX(queue_order), 0) AS value FROM jobs WHERE state = 'QUEUED' UNION ALL SELECT COALESCE(MAX(queue_order), 0) AS value FROM flow_definitions WHERE committed = 1)", [], |row| row.get(0))?)
}

pub(super) fn insert_occurrence(
    transaction: &Connection,
    definition: &FlowDefinition,
    local_date: Option<NaiveDate>,
    due_at: DateTime<Utc>,
    state: OccurrenceState,
    reason: Option<&str>,
) -> Result<(), StoreError> {
    let occurrence_id = Uuid::new_v4();
    let (kind, key, local_date) = match definition.schedule.as_ref() {
        Some(ScheduleSpec::Once { .. }) => ("once", "once".to_owned(), None),
        Some(ScheduleSpec::Daily { .. }) => {
            let date = local_date.ok_or_else(|| {
                StoreError::InvalidData("daily occurrence requires a local date".into())
            })?;
            ("daily", date.to_string(), Some(date.to_string()))
        }
        Some(ScheduleSpec::Periodic { .. }) => ("periodic", due_at.to_rfc3339(), None),
        None => {
            return Err(StoreError::InvalidData(
                "cannot create an occurrence without a schedule".into(),
            ));
        }
    };
    transaction.execute("INSERT OR IGNORE INTO occurrences (occurrence_id, internal_definition_id, flow_id, schedule_generation, kind, occurrence_key, local_date, due_at, state, reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![occurrence_id.to_string(), definition.internal_id.to_string(), definition.flow_id, definition.schedule_generation, kind, key, local_date, due_at.to_rfc3339(), occurrence_state_string(state), reason, Utc::now().to_rfc3339()])?;
    Ok(())
}

pub(super) fn materialize_daily_occurrences(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let rows = transaction.prepare("SELECT flow_id, internal_definition_id, schedule_generation, daily_time, schedule_timezone, committed_at, schedule_effective_from, daily_cursor_date FROM flow_definitions WHERE committed = 1 AND mode = 'scheduled'")?.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?,row.get::<_, String>(3).ok(),row.get::<_, String>(4).ok(),row.get::<_, String>(5).ok(),row.get::<_, String>(6).ok(),row.get::<_, String>(7).ok())))?.collect::<Result<Vec<_>, _>>()?;
    for (flow_id, internal_id, generation, time, timezone, committed_at, effective_from, cursor) in
        rows
    {
        let (Some(time), Some(timezone)) = (time, timezone) else {
            continue;
        };
        let zone: chrono_tz::Tz = match timezone.parse() {
            Ok(zone) => zone,
            Err(_) => continue,
        };
        let local = now.with_timezone(&zone);
        let wall_time = NaiveTime::parse_from_str(&time, "%H:%M")
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        let today = local.date_naive();
        let mut start = cursor
            .as_deref()
            .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
            .transpose()
            .map_err(|error| StoreError::InvalidData(error.to_string()))?
            .and_then(|date| date.succ_opt())
            .unwrap_or_else(|| {
                committed_at
                    .as_deref()
                    .and_then(|value| parse_datetime(value).ok())
                    .map(|instant| instant.with_timezone(&zone).date_naive())
                    .unwrap_or(today)
            });
        if let Some(value) = effective_from.as_deref() {
            let effective = NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|error| StoreError::InvalidData(error.to_string()))?;
            if start < effective {
                start = effective;
            }
        }
        let oldest = today - chrono::Duration::days(370);
        if start < oldest {
            start = oldest;
        }
        let mut date = start;
        let mut processed_through = cursor
            .as_deref()
            .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
            .transpose()
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        while date <= today {
            let date_key = date.to_string();
            let due = match zone.from_local_datetime(&date.and_time(wall_time)) {
                chrono::LocalResult::None => None,
                chrono::LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
                chrono::LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
            };
            if let Some(due) = due {
                if committed_at
                    .as_deref()
                    .is_some_and(|value| parse_datetime(value).map(|at| at > due).unwrap_or(false))
                {
                    processed_through = Some(date);
                } else if due > now {
                    break;
                } else if !occurrence_exists(
                    transaction,
                    &internal_id,
                    generation,
                    "daily",
                    &date_key,
                )? {
                    let (state, reason) = if due + chrono::Duration::seconds(5) < now {
                        (OccurrenceState::Skipped, Some("SKIPPED_MISSED"))
                    } else {
                        (OccurrenceState::Pending, None)
                    };
                    let event = if state == OccurrenceState::Pending {
                        "OCCURRENCE_PENDING"
                    } else {
                        "OCCURRENCE_SKIPPED"
                    };
                    let definition = load_flow_base(transaction, &flow_id)?;
                    insert_occurrence(transaction, &definition, Some(date), due, state, reason)?;
                    transaction.execute(
                        "INSERT INTO schedule_events (flow_id, internal_definition_id, schedule_generation, event, detail, occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            flow_id,
                            internal_id,
                            generation,
                            event,
                            reason,
                            now.to_rfc3339(),
                        ],
                    )?;
                    processed_through = Some(date);
                } else {
                    processed_through = Some(date);
                }
            } else if !occurrence_exists(transaction, &internal_id, generation, "daily", &date_key)?
            {
                let definition = load_flow_base(transaction, &flow_id)?;
                insert_occurrence(
                    transaction,
                    &definition,
                    Some(date),
                    now,
                    OccurrenceState::Skipped,
                    Some("SKIPPED_NONEXISTENT_TIME"),
                )?;
                transaction.execute(
                    "INSERT INTO schedule_events (flow_id, internal_definition_id, schedule_generation, event, detail, occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        flow_id,
                        internal_id,
                        generation,
                        "OCCURRENCE_SKIPPED",
                        "SKIPPED_NONEXISTENT_TIME",
                        now.to_rfc3339(),
                    ],
                )?;
                processed_through = Some(date);
            }
            date = date
                .succ_opt()
                .ok_or_else(|| StoreError::InvalidData("schedule date overflow".into()))?;
        }
        if let Some(processed_through) = processed_through {
            transaction.execute(
                "UPDATE flow_definitions SET daily_cursor_date = ?2 WHERE flow_id = ?1 AND schedule_generation = ?3",
                params![flow_id, processed_through.to_string(), generation],
            )?;
        }
    }
    Ok(())
}

pub(super) fn materialize_periodic_occurrences(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let flow_ids = transaction
        .prepare("SELECT flow_id FROM flow_definitions WHERE committed = 1 AND mode = 'scheduled' AND schedule_kind = 'periodic'")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for flow_id in flow_ids {
        let definition = load_flow_base(transaction, &flow_id)?;
        let Some(schedule @ ScheduleSpec::Periodic { .. }) = definition.schedule.as_ref() else {
            continue;
        };
        let (current, next) = schedule
            .periodic_window(now)
            .map_err(StoreError::InvalidData)?;
        if let Some(due) = current {
            let (state, reason) = if due + chrono::Duration::seconds(5) < now {
                (OccurrenceState::Skipped, Some("SKIPPED_MISSED"))
            } else {
                (OccurrenceState::Pending, None)
            };
            insert_periodic_occurrence_if_missing(transaction, &definition, due, state, reason)?;
        }
        insert_periodic_occurrence_if_missing(
            transaction,
            &definition,
            next,
            OccurrenceState::Pending,
            None,
        )?;
    }
    Ok(())
}

fn insert_periodic_occurrence_if_missing(
    transaction: &Transaction<'_>,
    definition: &FlowDefinition,
    due_at: DateTime<Utc>,
    state: OccurrenceState,
    reason: Option<&str>,
) -> Result<(), StoreError> {
    let key = due_at.to_rfc3339();
    if occurrence_exists(
        transaction,
        &definition.internal_id.to_string(),
        definition.schedule_generation,
        "periodic",
        &key,
    )? {
        return Ok(());
    }
    insert_occurrence(transaction, definition, None, due_at, state, reason)?;
    transaction.execute(
        "INSERT INTO schedule_events (flow_id, internal_definition_id, schedule_generation, event, detail, occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            definition.flow_id,
            definition.internal_id.to_string(),
            definition.schedule_generation,
            if state == OccurrenceState::Pending {
                "OCCURRENCE_PENDING"
            } else {
                "OCCURRENCE_SKIPPED"
            },
            reason,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub(super) fn occurrence_exists(
    transaction: &Transaction<'_>,
    internal_id: &str,
    generation: i64,
    kind: &str,
    key: &str,
) -> Result<bool, StoreError> {
    Ok(transaction.query_row("SELECT EXISTS(SELECT 1 FROM occurrences WHERE internal_definition_id = ?1 AND schedule_generation = ?2 AND kind = ?3 AND occurrence_key = ?4)", params![internal_id, generation, kind, key], |row| row.get(0))?)
}
pub(super) fn expire_occurrences(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    transaction.execute("UPDATE occurrences SET state = 'EXPIRED', reason = 'EXPIRED' WHERE kind = 'once' AND state = 'PENDING' AND due_at < ?1", [(now - chrono::Duration::hours(24)).to_rfc3339()])?;
    transaction.execute("UPDATE occurrences SET state = 'SKIPPED', reason = 'SKIPPED_MISSED' WHERE kind = 'daily' AND state = 'PENDING' AND due_at < ?1", [(now - chrono::Duration::seconds(5)).to_rfc3339()])?;
    transaction.execute("UPDATE occurrences SET state = 'SKIPPED', reason = 'SKIPPED_MISSED' WHERE kind = 'periodic' AND state = 'PENDING' AND due_at < ?1", [(now - chrono::Duration::seconds(5)).to_rfc3339()])?;
    Ok(())
}

pub(super) fn schedule_from_row(
    row: &Row<'_>,
    index: usize,
) -> Result<Option<ScheduleSpec>, String> {
    let kind: Option<String> = row.get(index).map_err(|error| error.to_string())?;
    match kind.as_deref() {
        None => Ok(None),
        Some("once") => Ok(Some(ScheduleSpec::Once {
            at: parse_datetime(
                &row.get::<_, String>(index + 1)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?,
        })),
        Some("daily") => Ok(Some(ScheduleSpec::Daily {
            time: NaiveTime::parse_from_str(
                &row.get::<_, String>(index + 2)
                    .map_err(|error| error.to_string())?,
                "%H:%M",
            )
            .map_err(|error| error.to_string())?,
            timezone: row.get(index + 3).map_err(|error| error.to_string())?,
        })),
        Some("periodic") => Ok(Some(ScheduleSpec::Periodic {
            every: SchedulePeriod {
                value: u32::try_from(
                    row.get::<_, i64>(index + 4)
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|_| "invalid periodic schedule value".to_owned())?,
                unit: match row
                    .get::<_, String>(index + 5)
                    .map_err(|error| error.to_string())?
                    .as_str()
                {
                    "minutes" => SchedulePeriodUnit::Minutes,
                    "hours" => SchedulePeriodUnit::Hours,
                    other => return Err(format!("unknown periodic schedule unit {other}")),
                },
            },
            first_at: row
                .get::<_, Option<String>>(index + 6)
                .map_err(|error| error.to_string())?
                .as_deref()
                .map(parse_datetime)
                .transpose()
                .map_err(|error| error.to_string())?,
        })),
        Some(other) => Err(format!("unknown schedule kind {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Store;
    use crate::domain::flow::DependencyMode;
    use crate::store::FlowTaskInput;

    fn daily_store(
        root: &std::path::Path,
        flow_id: &str,
        time: NaiveTime,
        timezone: &str,
    ) -> Store {
        let store = Store::open(root.join(format!("{flow_id}.db"))).unwrap();
        store
            .create_flow(
                flow_id.into(),
                flow_id.into(),
                "tester".into(),
                ScheduleSpec::Daily {
                    time,
                    timezone: timezone.into(),
                },
            )
            .unwrap();
        store
            .add_flow_task(FlowTaskInput {
                flow_id: flow_id.into(),
                task_id: "root".into(),
                name: "Root".into(),
                cwd: root.to_string_lossy().into_owned(),
                command: "echo daily".into(),
                retry: 0,
                dependencies: vec![],
                depend_mode: DependencyMode::All,
            })
            .unwrap();
        store.commit_flow(flow_id).unwrap();
        store
    }

    #[test]
    fn daily_materialization_records_missed_due_and_nonexistent_local_times() {
        let directory = tempfile::tempdir().unwrap();
        let store = daily_store(
            directory.path(),
            "daily-utc",
            NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            "UTC",
        );
        let now = DateTime::parse_from_rfc3339("2026-09-23T08:30:02Z")
            .unwrap()
            .with_timezone(&Utc);
        {
            let mut connection = store.lock().unwrap();
            connection.execute("UPDATE flow_definitions SET committed_at = '2026-09-20T09:00:00+00:00', daily_cursor_date = '2026-09-19' WHERE flow_id = 'daily-utc'", []).unwrap();
            let transaction = connection.transaction().unwrap();
            materialize_daily_occurrences(&transaction, now).unwrap();
            materialize_daily_occurrences(&transaction, now).unwrap();
            transaction.commit().unwrap();
            let rows = connection
                .prepare("SELECT local_date, state, reason FROM occurrences ORDER BY local_date")
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(rows.len(), 3);
            assert_eq!(
                rows[0],
                (
                    "2026-09-21".into(),
                    "SKIPPED".into(),
                    Some("SKIPPED_MISSED".into())
                )
            );
            assert_eq!(rows[1].0, "2026-09-22");
            assert_eq!(rows[2], ("2026-09-23".into(), "PENDING".into(), None));
            let cursor: String = connection
                .query_row(
                    "SELECT daily_cursor_date FROM flow_definitions WHERE flow_id = 'daily-utc'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(cursor, "2026-09-23");
        }

        let store = daily_store(
            directory.path(),
            "dst-gap",
            NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
            "America/New_York",
        );
        let now = DateTime::parse_from_rfc3339("2026-03-08T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut connection = store.lock().unwrap();
        connection.execute("UPDATE flow_definitions SET committed_at = '2026-03-01T00:00:00+00:00', schedule_effective_from = '2026-03-08', daily_cursor_date = '2026-03-07' WHERE flow_id = 'dst-gap'", []).unwrap();
        let transaction = connection.transaction().unwrap();
        materialize_daily_occurrences(&transaction, now).unwrap();
        transaction.commit().unwrap();
        let row: (String, String) = connection
            .query_row(
                "SELECT state, reason FROM occurrences WHERE flow_id = 'dst-gap'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("SKIPPED".into(), "SKIPPED_NONEXISTENT_TIME".into()));
    }

    #[test]
    fn daily_materialization_reports_invalid_persisted_schedule_fields() {
        let directory = tempfile::tempdir().unwrap();
        let store = daily_store(
            directory.path(),
            "bad-daily",
            NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            "UTC",
        );
        let now = DateTime::parse_from_rfc3339("2026-09-23T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut connection = store.lock().unwrap();
        for (column, value) in [
            ("daily_time", "bad-time"),
            ("daily_cursor_date", "bad-date"),
            ("schedule_effective_from", "bad-date"),
        ] {
            connection.execute("UPDATE flow_definitions SET daily_time = '08:30', daily_cursor_date = NULL, schedule_effective_from = NULL WHERE flow_id = 'bad-daily'", []).unwrap();
            connection
                .execute(
                    &format!(
                        "UPDATE flow_definitions SET {column} = ?1 WHERE flow_id = 'bad-daily'"
                    ),
                    [value],
                )
                .unwrap();
            let transaction = connection.transaction().unwrap();
            let error = materialize_daily_occurrences(&transaction, now).unwrap_err();
            assert!(
                matches!(&error, StoreError::InvalidData(_)),
                "{column}: {error}"
            );
        }
        connection.execute("UPDATE flow_definitions SET daily_time = '08:30', daily_cursor_date = NULL, schedule_effective_from = NULL, schedule_timezone = 'Not/A_Zone' WHERE flow_id = 'bad-daily'", []).unwrap();
        let transaction = connection.transaction().unwrap();
        materialize_daily_occurrences(&transaction, now).unwrap();
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM occurrences", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    fn job_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                    id TEXT PRIMARY KEY, state TEXT, queue_order INTEGER, started_at TEXT,
                    finished_at TEXT, exit_code INTEGER, failure_detail TEXT,
                    schedule_kind TEXT
                );",
            )
            .unwrap();
        connection
    }

    fn insert_job(connection: &Connection, id: Uuid, kind: &str, state: &str, order: i64) {
        connection
            .execute(
                "INSERT INTO jobs (id,state,queue_order,started_at,finished_at,exit_code,failure_detail,schedule_kind)
                 VALUES (?1,?2,?3,'old-start','old-finish',7,'old-failure',?4)",
                params![id.to_string(), state, order, kind],
            )
            .unwrap();
    }

    #[test]
    fn standalone_job_state_sync_handles_daily_requeue_terminal_states_and_non_standalone_ids() {
        let connection = job_connection();
        let daily = Uuid::new_v4();
        let once = Uuid::new_v4();
        let queued = Uuid::new_v4();
        insert_job(&connection, daily, "daily", "QUEUED", 1);
        insert_job(&connection, once, "once", "STARTING", 2);
        insert_job(&connection, queued, "once", "QUEUED", 4);

        sync_standalone_job_start(&connection, &format!("standalone/{daily}")).unwrap();
        let (state, order, started): (String, Option<i64>, Option<String>) = connection
            .query_row(
                "SELECT state,queue_order,started_at FROM jobs WHERE id=?1",
                [daily.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "STARTING");
        assert_eq!(order, None);
        assert!(started.is_some());

        sync_standalone_job_running(&connection, &format!("standalone/{daily}")).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM jobs WHERE id=?1",
                    [daily.to_string()],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "RUNNING"
        );

        sync_standalone_job_terminal(&connection, &format!("standalone/{daily}"), "FAILED")
            .unwrap();
        let daily_row: (String, i64, Option<String>, Option<String>, Option<i32>, Option<String>) =
            connection
                .query_row(
                    "SELECT state,queue_order,started_at,finished_at,exit_code,failure_detail FROM jobs WHERE id=?1",
                    [daily.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
                )
                .unwrap();
        assert_eq!(daily_row, ("QUEUED".into(), 5, None, None, None, None));

        for (state, expected) in [
            ("SUCCEEDED", "SUCCEEDED"),
            ("CANCELLED", "CANCELLED"),
            ("FAILED", "FAILED"),
            ("LOST", "FAILED"),
        ] {
            sync_standalone_job_terminal(&connection, &format!("standalone/{once}"), state)
                .unwrap();
            let actual: String = connection
                .query_row(
                    "SELECT state FROM jobs WHERE id=?1",
                    [once.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(actual, expected);
            connection
                .execute(
                    "UPDATE jobs SET state='STARTING',queue_order=2 WHERE id=?1",
                    [once.to_string()],
                )
                .unwrap();
        }

        for flow_id in ["user-flow", "standalone/not-a-uuid", "standalone/a/b"] {
            sync_standalone_job_start(&connection, flow_id).unwrap();
            sync_standalone_job_running(&connection, flow_id).unwrap();
            sync_standalone_job_terminal(&connection, flow_id, "FAILED").unwrap();
        }
    }

    #[test]
    fn schedule_row_mapping_covers_schedule_variants_and_bad_rows() {
        let once = DateTime::parse_from_rfc3339("2026-09-23T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let cases = [
            (
                "once",
                Some("2026-09-23T00:00:00Z"),
                "08:30",
                "Asia/Tokyo",
                15_i64,
                "minutes",
                None::<&str>,
            ),
            ("daily", None, "08:30", "Asia/Tokyo", 15, "minutes", None),
            (
                "periodic",
                None,
                "08:30",
                "Asia/Tokyo",
                2,
                "hours",
                Some("2026-09-23T00:00:00Z"),
            ),
            ("periodic", None, "08:30", "Asia/Tokyo", 2, "minutes", None),
        ];
        for (kind, at, time, timezone, period, unit, expected_first_at) in cases {
            let schedule = Connection::open_in_memory()
                .unwrap()
                .query_row(
                    "SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7",
                    params![kind, at, time, timezone, period, unit, expected_first_at],
                    |row| Ok(schedule_from_row(row, 0)),
                )
                .unwrap()
                .unwrap();
            match (kind, schedule) {
                ("once", Some(ScheduleSpec::Once { at })) => assert_eq!(at, once),
                ("daily", Some(ScheduleSpec::Daily { time, timezone })) => {
                    assert_eq!(time, NaiveTime::from_hms_opt(8, 30, 0).unwrap());
                    assert_eq!(timezone, "Asia/Tokyo");
                }
                ("periodic", Some(ScheduleSpec::Periodic { every, first_at })) => {
                    assert_eq!(every.value, period as u32);
                    assert_eq!(
                        every.unit,
                        match unit {
                            "hours" => SchedulePeriodUnit::Hours,
                            _ => SchedulePeriodUnit::Minutes,
                        }
                    );
                    assert_eq!(first_at.is_some(), expected_first_at.is_some());
                }
                _ => panic!("unexpected schedule mapping for {kind}"),
            }
        }

        for (kind, at, time, timezone, period, unit, first_at) in [
            ("unknown", None, "08:30", "UTC", 1_i64, "minutes", None),
            ("once", Some("bad-time"), "08:30", "UTC", 1, "minutes", None),
            ("daily", None, "bad-time", "UTC", 1, "minutes", None),
            ("periodic", None, "08:30", "UTC", -1, "minutes", None),
            ("periodic", None, "08:30", "UTC", 1, "days", None),
            (
                "periodic",
                None,
                "08:30",
                "UTC",
                1,
                "minutes",
                Some("bad-time"),
            ),
        ] {
            let result = Connection::open_in_memory()
                .unwrap()
                .query_row(
                    "SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7",
                    params![kind, at, time, timezone, period, unit, first_at],
                    |row| Ok(schedule_from_row(row, 0)),
                )
                .unwrap();
            assert!(result.is_err(), "{kind} {time} {unit}");
        }
        let absent = Connection::open_in_memory()
            .unwrap()
            .query_row("SELECT NULL", [], |row| Ok(schedule_from_row(row, 0)))
            .unwrap();
        assert_eq!(absent.unwrap(), None);
    }
}
