//! Runtime and schedule row helpers for flow persistence.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

use crate::domain::flow::{
    ExecutionMode, FlowDefinition, OccurrenceState, ScheduleSpec, TaskRunState,
};

use super::error::StoreError;
use super::flow_mapping::*;

pub(super) fn has_active_run(transaction: &Connection, flow_id: &str) -> Result<bool, StoreError> {
    Ok(transaction.query_row("SELECT EXISTS(SELECT 1 FROM flow_runs WHERE flow_id = ?1 AND state IN ('STARTING','RUNNING','CANCELLING','RECOVERING'))", [flow_id], |row| row.get(0))?)
}

pub(super) fn settle_daily_occurrences(
    transaction: &Connection,
    now: DateTime<Utc>,
    capacity_full: bool,
    gate_reason: Option<&str>,
) -> Result<(), StoreError> {
    let workspace_mode = parse_mode(&transaction.query_row(
        "SELECT mode FROM settings WHERE id = 1",
        [],
        |row| row.get::<_, String>(0),
    )?)?;
    let rows = transaction
        .prepare("SELECT occurrence_id, flow_id FROM occurrences WHERE kind = 'daily' AND state = 'PENDING' AND due_at <= ?1 AND due_at >= ?2 ORDER BY due_at, occurrence_id")?
        .query_map(
            params![
                now.to_rfc3339(),
                (now - chrono::Duration::seconds(5)).to_rfc3339()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    for (occurrence_id, flow_id) in rows {
        if let Some(reason) = gate_reason {
            transaction.execute(
                "UPDATE occurrences SET state = 'SKIPPED', reason = ?2 WHERE occurrence_id = ?1 AND state = 'PENDING'",
                params![occurrence_id, reason],
            )?;
            continue;
        }
        let definition = load_flow_base(transaction, &flow_id)?;
        let reason = if !definition.enabled {
            Some("SKIPPED_DISABLED")
        } else if workspace_mode != ExecutionMode::Scheduled
            || definition.mode != ExecutionMode::Scheduled
        {
            Some("SKIPPED_MODE")
        } else if definition.frozen {
            Some("SKIPPED_FROZEN")
        } else if has_active_run(transaction, &flow_id)? {
            Some("SKIPPED_OVERLAP")
        } else if capacity_full {
            Some("SKIPPED_CAPACITY")
        } else {
            None
        };
        if let Some(reason) = reason {
            transaction.execute(
                "UPDATE occurrences SET state = 'SKIPPED', reason = ?2 WHERE occurrence_id = ?1 AND state = 'PENDING'",
                params![occurrence_id, reason],
            )?;
        }
    }
    Ok(())
}

/// Claim one due scheduled occurrence as a new run while holding the queue
/// transaction.  This is the single-writer fence that prevents duplicate
/// automatic runs when several scheduler ticks race on the same database.
pub(super) fn start_due_flow_run(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
    mode: ExecutionMode,
) -> Result<(), StoreError> {
    let candidates = transaction.prepare("SELECT o.occurrence_id, o.flow_id FROM occurrences o JOIN flow_definitions f ON f.flow_id = o.flow_id WHERE o.state = 'PENDING' AND o.due_at <= ?1 AND o.due_at >= ?2 ORDER BY o.due_at, f.last_dispatch_sequence, f.committed_at, f.internal_definition_id")?.query_map(params![now.to_rfc3339(), (now - chrono::Duration::hours(24)).to_rfc3339()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    for (occurrence_id, flow_id) in candidates {
        let definition = load_flow_base(transaction, &flow_id)?;
        if definition.mode != mode
            || definition.mode != ExecutionMode::Scheduled
            || !definition.committed
            || definition.frozen
            || !definition.enabled
            || has_active_run(transaction, &flow_id)?
        {
            continue;
        }
        let run_id = Uuid::new_v4();
        transaction.execute("INSERT INTO flow_runs (run_id, flow_id, internal_definition_id, schedule_generation, source, state, occurrence_id, definition_snapshot, created_at) VALUES (?1, ?2, ?3, ?4, 'AUTOMATIC', 'STARTING', ?5, ?6, ?7)", params![run_id.to_string(), flow_id, definition.internal_id.to_string(), definition.schedule_generation, occurrence_id, serde_json::to_string(&definition)?, now.to_rfc3339()])?;
        for task in &definition.tasks {
            let state = if task.dependencies.is_empty() {
                TaskRunState::Ready
            } else {
                TaskRunState::Waiting
            };
            transaction.execute(
                "INSERT INTO task_runs (run_id, task_id, state) VALUES (?1, ?2, ?3)",
                params![run_id.to_string(), task.task_id, task_state_string(state)],
            )?;
        }
        transaction.execute("UPDATE occurrences SET state = 'RESERVED' WHERE occurrence_id = ?1 AND state = 'PENDING'", [&occurrence_id])?;
        let sequence: i64 = transaction.query_row(
            "SELECT dispatch_sequence + 1 FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE settings SET dispatch_sequence = ?1 WHERE id = 1",
            [sequence],
        )?;
        transaction.execute(
            "UPDATE flow_definitions SET last_dispatch_sequence = ?2 WHERE flow_id = ?1",
            params![flow_id, sequence],
        )?;
        sync_standalone_job_start(transaction, &flow_id)?;
        break;
    }
    Ok(())
}

pub(super) fn start_serial_standalone_run(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let candidate: Option<(String, String)> = transaction
        .query_row(
            "SELECT j.id, f.flow_id FROM jobs j JOIN flow_definitions f ON f.flow_id = 'standalone/' || j.id WHERE j.state = 'QUEUED' AND j.mode = 'serial' AND j.retry > 0 AND f.committed = 1 AND f.enabled = 1 ORDER BY j.queue_order, j.id LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((job_id, flow_id)) = candidate else {
        return Ok(());
    };
    if has_active_run(transaction, &flow_id)? {
        return Ok(());
    }
    let definition = load_flow_base(transaction, &flow_id)?;
    let run_id = Uuid::new_v4();
    let now = Utc::now();
    transaction.execute(
        "INSERT INTO flow_runs (run_id, flow_id, internal_definition_id, schedule_generation, source, state, definition_snapshot, created_at) VALUES (?1, ?2, ?3, ?4, 'AUTOMATIC', 'STARTING', ?5, ?6)",
        params![
            run_id.to_string(),
            flow_id,
            definition.internal_id.to_string(),
            definition.schedule_generation,
            serde_json::to_string(&definition)?,
            now.to_rfc3339(),
        ],
    )?;
    for task in &definition.tasks {
        transaction.execute(
            "INSERT INTO task_runs (run_id, task_id, state) VALUES (?1, ?2, ?3)",
            params![
                run_id.to_string(),
                task.task_id,
                task_state_string(TaskRunState::Ready)
            ],
        )?;
    }
    sync_standalone_job_start(transaction, &format!("standalone/{job_id}"))?;
    Ok(())
}

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
    let row: Option<String> = transaction.query_row("SELECT occurrence_id FROM occurrences WHERE flow_id = ?1 AND schedule_generation = ?2 AND state = 'PENDING' AND ((kind = 'daily' AND due_at > ?3) OR (kind = 'once' AND due_at > ?4)) ORDER BY due_at LIMIT 1", params![definition.flow_id, definition.schedule_generation, now.to_rfc3339(), (now - chrono::Duration::hours(24)).to_rfc3339()], |row| row.get(0)).optional()?;
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
    let key = local_date
        .map(|date| date.to_string())
        .unwrap_or_else(|| "once".into());
    transaction.execute("INSERT OR IGNORE INTO occurrences (occurrence_id, internal_definition_id, flow_id, schedule_generation, kind, occurrence_key, local_date, due_at, state, reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![occurrence_id.to_string(), definition.internal_id.to_string(), definition.flow_id, definition.schedule_generation, if local_date.is_some() { "daily" } else { "once" }, key, local_date.map(|date| date.to_string()), due_at.to_rfc3339(), occurrence_state_string(state), reason, Utc::now().to_rfc3339()])?;
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
                } else if !occurrence_exists(transaction, &internal_id, generation, &date_key)? {
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
            } else if !occurrence_exists(transaction, &internal_id, generation, &date_key)? {
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

pub(super) fn occurrence_exists(
    transaction: &Transaction<'_>,
    internal_id: &str,
    generation: i64,
    key: &str,
) -> Result<bool, StoreError> {
    Ok(transaction.query_row("SELECT EXISTS(SELECT 1 FROM occurrences WHERE internal_definition_id = ?1 AND schedule_generation = ?2 AND kind = 'daily' AND occurrence_key = ?3)", params![internal_id, generation, key], |row| row.get(0))?)
}
pub(super) fn expire_occurrences(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    transaction.execute("UPDATE occurrences SET state = 'EXPIRED', reason = 'EXPIRED' WHERE kind = 'once' AND state = 'PENDING' AND due_at < ?1", [(now - chrono::Duration::hours(24)).to_rfc3339()])?;
    transaction.execute("UPDATE occurrences SET state = 'SKIPPED', reason = 'SKIPPED_MISSED' WHERE kind = 'daily' AND state = 'PENDING' AND due_at < ?1", [(now - chrono::Duration::seconds(5)).to_rfc3339()])?;
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
        Some(other) => Err(format!("unknown schedule kind {other}")),
    }
}
