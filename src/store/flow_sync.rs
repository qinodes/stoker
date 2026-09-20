//! Atomic desired-state reconciliation for declarative Flow sources.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::Utc;
use rusqlite::{Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::flow::{ExecutionMode, FlowDefinition, OccurrenceState, ScheduleSpec};
use crate::flow_source::{FlowSourceDocument, archive_source, canonical_hash, write_snapshot};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{has_active_work, insert_task, parse_mode, require_queue_locked};
use super::flow_runtime_mapping::insert_occurrence;
use super::flow_schedule_mapping::schedule_columns;
use super::flow_sources::{
    FlowSourceMode, hash_committed_definitions, load_committed_definitions, source_state_with,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlowSyncDiff {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowSyncResult {
    pub diff: FlowSyncDiff,
    pub revision: i64,
    pub hash: String,
    pub changed: bool,
}

impl Store {
    pub fn sync_flow_source(
        &self,
        root: &Path,
        source: &FlowSourceDocument,
        desired: &[FlowDefinition],
        dry_run: bool,
    ) -> Result<FlowSyncResult, StoreError> {
        let desired_document = FlowSourceDocument::from_definitions(0, desired)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        let desired_hash = desired_document.base.hash;
        let source_hash =
            canonical_hash(source).map_err(|error| StoreError::InvalidData(error.to_string()))?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_sync_gates(&transaction)?;
        let (_, revision, _) = source_state_with(&transaction)?;
        let current = load_committed_definitions(&transaction)?;
        let current_hash = hash_committed_definitions(&transaction, revision)?;
        let diff = calculate_diff(&current, desired);

        if desired_hash == current_hash {
            transaction.commit()?;
            return Ok(FlowSyncResult {
                diff,
                revision,
                hash: current_hash,
                changed: false,
            });
        }
        if source.base.revision != revision || source.base.hash != current_hash {
            return Err(StoreError::InvalidData(format!(
                "stale Flow definition: source base is revision {} {}, current workspace is revision {} {}; run `stoker flow export` and reapply your changes",
                source.base.revision, source.base.hash, revision, current_hash
            )));
        }
        if dry_run {
            transaction.commit()?;
            return Ok(FlowSyncResult {
                diff,
                revision,
                hash: desired_hash,
                changed: true,
            });
        }

        let snapshot = FlowSourceDocument::from_definitions(revision, &current)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        write_snapshot(root, &snapshot)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        archive_source(root, source).map_err(|error| StoreError::InvalidData(error.to_string()))?;
        reconcile(&transaction, &current, desired)?;
        let next = revision
            .checked_add(1)
            .ok_or_else(|| StoreError::InvalidData("definition revision overflow".into()))?;
        transaction.execute(
            "UPDATE settings SET flow_definition_revision = ?1, flow_definition_hash = ?2, flow_last_sync_at = ?3, flow_last_source_hash = ?4 WHERE id = 1",
            params![next, desired_hash, Utc::now().to_rfc3339(), source_hash],
        )?;
        transaction.execute(
            "INSERT INTO flow_sync_audit (revision, previous_hash, source_hash, synced_at, added, updated, removed, unchanged) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![next, current_hash, source_hash, Utc::now().to_rfc3339(), diff.added as i64, diff.updated as i64, diff.removed as i64, diff.unchanged as i64],
        )?;
        transaction.commit()?;
        Ok(FlowSyncResult {
            diff,
            revision: next,
            hash: desired_hash,
            changed: true,
        })
    }
}

fn require_sync_gates(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let execution: String =
        transaction.query_row("SELECT mode FROM settings WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    if parse_mode(&execution)? != ExecutionMode::Scheduled {
        return Err(StoreError::InvalidData(
            "flow sync is only available in scheduled execution mode".into(),
        ));
    }
    let (source_mode, _, _) = source_state_with(transaction)?;
    if source_mode != FlowSourceMode::Sync {
        return Err(StoreError::InvalidData("flow sync requires source mode sync; lock the queue and run `stoker flow source-mode sync`".into()));
    }
    require_queue_locked(transaction)?;
    if has_active_work(transaction)? {
        return Err(StoreError::InvalidData(
            "cannot sync while work is active or recovery is incomplete".into(),
        ));
    }
    Ok(())
}

fn calculate_diff(current: &[FlowDefinition], desired: &[FlowDefinition]) -> FlowSyncDiff {
    let current_by_id = current
        .iter()
        .enumerate()
        .map(|(index, flow)| (flow.flow_id.as_str(), (index, flow)))
        .collect::<BTreeMap<_, _>>();
    let desired_ids = desired
        .iter()
        .map(|flow| flow.flow_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut diff = FlowSyncDiff {
        removed: current_by_id
            .keys()
            .filter(|id| !desired_ids.contains(**id))
            .count(),
        ..FlowSyncDiff::default()
    };
    for (index, flow) in desired.iter().enumerate() {
        match current_by_id.get(flow.flow_id.as_str()) {
            None => diff.added += 1,
            Some((current_index, existing))
                if *current_index == index && same_desired_state(existing, flow) =>
            {
                diff.unchanged += 1
            }
            Some(_) => diff.updated += 1,
        }
    }
    diff
}

fn same_desired_state(current: &FlowDefinition, desired: &FlowDefinition) -> bool {
    current.name == desired.name
        && current.owner == desired.owner
        && current.enabled == desired.enabled
        && current.schedule == desired.schedule
        && current.tasks == desired.tasks
}

fn reconcile(
    transaction: &Transaction<'_>,
    current: &[FlowDefinition],
    desired: &[FlowDefinition],
) -> Result<(), StoreError> {
    let desired_ids = desired
        .iter()
        .map(|flow| flow.flow_id.as_str())
        .collect::<BTreeSet<_>>();
    for flow in current
        .iter()
        .filter(|flow| !desired_ids.contains(flow.flow_id.as_str()))
    {
        transaction.execute("UPDATE occurrences SET state = 'SUPERSEDED', reason = 'definition removed by source sync' WHERE flow_id = ?1 AND state IN ('PENDING','RESERVED')", [&flow.flow_id])?;
        transaction.execute(
            "DELETE FROM flow_dependencies WHERE flow_id = ?1",
            [&flow.flow_id],
        )?;
        transaction.execute("DELETE FROM flow_tasks WHERE flow_id = ?1", [&flow.flow_id])?;
        transaction.execute(
            "DELETE FROM flow_definitions WHERE flow_id = ?1",
            [&flow.flow_id],
        )?;
    }
    let current_by_id = current
        .iter()
        .map(|flow| (flow.flow_id.as_str(), flow))
        .collect::<BTreeMap<_, _>>();
    for (index, desired_flow) in desired.iter().enumerate() {
        if let Some(existing) = current_by_id.get(desired_flow.flow_id.as_str()) {
            update_flow(transaction, existing, desired_flow, index as i64 + 1)?;
        } else {
            insert_flow(transaction, desired_flow, index as i64 + 1)?;
        }
    }
    Ok(())
}

fn insert_flow(
    transaction: &Transaction<'_>,
    desired: &FlowDefinition,
    order: i64,
) -> Result<(), StoreError> {
    let now = Utc::now();
    let internal = Uuid::new_v4();
    let columns = schedule_columns(desired.schedule.as_ref());
    transaction.execute("INSERT INTO flow_definitions (flow_id, internal_definition_id, name, owner, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, period_value, period_unit, period_first_at_utc, schedule_generation, committed, frozen, enabled, graph_revision, draft_revision, queue_order, created_at, committed_at, last_dispatch_sequence) VALUES (?1,?2,?3,?4,'scheduled',?5,?6,?7,?8,?9,?10,?11,1,1,0,?12,1,0,?13,?14,?14,0)", params![desired.flow_id, internal.to_string(), desired.name, desired.owner, columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, i64::from(desired.enabled), order, now.to_rfc3339()])?;
    for task in &desired.tasks {
        insert_task(transaction, &desired.flow_id, task)?;
    }
    let mut stored = desired.clone();
    stored.internal_id = internal;
    stored.graph_revision = 1;
    stored.schedule_generation = 1;
    insert_first_occurrence(transaction, &stored)?;
    Ok(())
}

fn update_flow(
    transaction: &Transaction<'_>,
    current: &FlowDefinition,
    desired: &FlowDefinition,
    order: i64,
) -> Result<(), StoreError> {
    let graph_changed = current.name != desired.name
        || current.owner != desired.owner
        || current.tasks != desired.tasks;
    let schedule_changed = current.schedule != desired.schedule;
    if !graph_changed
        && !schedule_changed
        && current.enabled == desired.enabled
        && current.queue_order == Some(order)
    {
        return Ok(());
    }
    if schedule_changed {
        transaction.execute("UPDATE occurrences SET state = 'SUPERSEDED', reason = 'schedule generation replaced by source sync' WHERE flow_id = ?1 AND state IN ('PENDING','RESERVED')", [&current.flow_id])?;
    }
    if graph_changed {
        transaction.execute(
            "DELETE FROM flow_dependencies WHERE flow_id = ?1",
            [&current.flow_id],
        )?;
        transaction.execute(
            "DELETE FROM flow_tasks WHERE flow_id = ?1",
            [&current.flow_id],
        )?;
        for task in &desired.tasks {
            insert_task(transaction, &desired.flow_id, task)?;
        }
    }
    let generation = current.schedule_generation + i64::from(schedule_changed);
    let graph_revision = current.graph_revision + i64::from(graph_changed);
    let columns = schedule_columns(desired.schedule.as_ref());
    transaction.execute("UPDATE flow_definitions SET name=?2, owner=?3, schedule_kind=?4, schedule_at_utc=?5, daily_time=?6, schedule_timezone=?7, period_value=?8, period_unit=?9, period_first_at_utc=?10, schedule_generation=?11, enabled=?12, graph_revision=?13, queue_order=?14 WHERE flow_id=?1", params![desired.flow_id, desired.name, desired.owner, columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, generation, i64::from(desired.enabled), graph_revision, order])?;
    if schedule_changed {
        let mut stored = desired.clone();
        stored.internal_id = current.internal_id;
        stored.graph_revision = graph_revision;
        stored.schedule_generation = generation;
        insert_first_occurrence(transaction, &stored)?;
    }
    Ok(())
}

fn insert_first_occurrence(
    transaction: &Transaction<'_>,
    flow: &FlowDefinition,
) -> Result<(), StoreError> {
    let first = match flow.schedule.as_ref() {
        Some(ScheduleSpec::Once { at }) => Some(*at),
        Some(ScheduleSpec::Periodic { first_at, .. }) => *first_at,
        _ => None,
    };
    if let Some(at) = first {
        insert_occurrence(transaction, flow, None, at, OccurrenceState::Pending, None)?;
    }
    Ok(())
}
