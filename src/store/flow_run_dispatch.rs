//! Atomic selection and dispatch of runnable flow occurrences.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::domain::flow::{ExecutionMode, TaskRunState};

use super::error::StoreError;
use super::flow_mapping::{load_flow_base, parse_mode, task_state_string};
use super::flow_runtime_mapping::sync_standalone_job_start;

pub(super) fn has_active_run(transaction: &Connection, flow_id: &str) -> Result<bool, StoreError> {
    Ok(transaction.query_row("SELECT EXISTS(SELECT 1 FROM flow_runs WHERE flow_id = ?1 AND state IN ('STARTING','RUNNING','CANCELLING','RECOVERING'))", [flow_id], |row| row.get(0))?)
}

pub(super) fn settle_recurring_occurrences(
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
        .prepare("SELECT occurrence_id, flow_id FROM occurrences WHERE kind IN ('daily','periodic') AND state = 'PENDING' AND due_at <= ?1 AND due_at >= ?2 ORDER BY due_at, occurrence_id")?
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
/// transaction. This is the single-writer fence that prevents duplicate
/// automatic runs when several scheduler ticks race on the same database.
pub(super) fn start_due_flow_run(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
    mode: ExecutionMode,
) -> Result<(), StoreError> {
    let candidates = transaction.prepare("SELECT o.occurrence_id, o.flow_id FROM occurrences o JOIN flow_definitions f ON f.flow_id = o.flow_id WHERE o.state = 'PENDING' AND o.due_at <= ?1 AND ((o.kind = 'once' AND o.due_at >= ?2) OR (o.kind IN ('daily','periodic') AND o.due_at >= ?3)) ORDER BY o.due_at, f.last_dispatch_sequence, f.committed_at, f.internal_definition_id")?.query_map(params![now.to_rfc3339(), (now - chrono::Duration::hours(24)).to_rfc3339(), (now - chrono::Duration::seconds(5)).to_rfc3339()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
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
