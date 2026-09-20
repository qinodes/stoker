//! SQLite persistence for flow definitions and execution history.

use std::collections::BTreeSet;

use chrono::Utc;
use rusqlite::{TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::flow::{
    Dependency, DependencyMode, ExecutionMode, FlowDefinition, FlowTask, OccurrenceState,
    ScheduleSpec, TaskRunState, aggregate_flow_state, validate_definition, validate_flow_metadata,
};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::*;
use super::flow_runtime_mapping::*;
use super::flow_schedule_mapping::*;
use super::flow_sources::{advance_definition_state, require_manual_source};
use super::standalone::sync_standalone_job_from_flow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowTaskInput {
    pub flow_id: String,
    pub task_id: String,
    pub name: String,
    pub cwd: String,
    pub command: String,
    pub retry: u32,
    pub dependencies: Vec<Dependency>,
    pub depend_mode: DependencyMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowTaskExecution {
    pub run_id: Uuid,
    pub flow_id: String,
    pub task_id: String,
    pub attempt_id: Uuid,
    pub attempt_number: u32,
    pub first_root: bool,
    pub task: FlowTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowAttemptResult {
    Succeeded {
        exit_code: i32,
    },
    Failed {
        exit_code: Option<i32>,
        kind: String,
        detail: String,
    },
    Cancelled {
        detail: String,
    },
    Lost {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandaloneDefinition {
    pub job_id: Uuid,
    pub mode: ExecutionMode,
    pub schedule: Option<ScheduleSpec>,
    pub retry: u32,
    pub enabled: bool,
    pub generation: i64,
}

impl Store {
    pub fn create_flow(
        &self,
        flow_id: String,
        name: String,
        owner: String,
        schedule: ScheduleSpec,
    ) -> Result<FlowDefinition, StoreError> {
        let mode = ExecutionMode::Scheduled;
        validate_flow_metadata(&flow_id, &name, &owner).map_err(StoreError::InvalidData)?;
        validate_schedule(mode, Some(&schedule))?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(&flow_id))?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM flow_definitions WHERE flow_id = ?1)",
            [&flow_id],
            |row| row.get(0),
        )?;
        if exists {
            return Err(StoreError::InvalidData(format!(
                "flow-id {flow_id:?} already exists"
            )));
        }
        let internal_id = Uuid::new_v4();
        let now = Utc::now();
        let columns = schedule_columns(Some(&schedule));
        transaction.execute(
            "INSERT INTO flow_definitions (flow_id, internal_definition_id, name, owner, mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, period_value, period_unit, period_first_at_utc, schedule_generation, committed, frozen, enabled, graph_revision, draft_revision, created_at, last_dispatch_sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, 0, 0, 1, 0, 0, ?13, 0)",
            params![flow_id, internal_id.to_string(), name, owner, mode.to_string(), columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, now.to_rfc3339()],
        )?;
        let definition = load_flow(&transaction, &flow_id)?;
        transaction.commit()?;
        Ok(definition)
    }

    pub fn commit_flow(&self, flow_id: &str) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(flow_id))?;
        require_queue_unlocked(&transaction)?;
        let mut definition = load_flow_current(&transaction, flow_id)?;
        if definition.committed {
            return Err(StoreError::InvalidData(format!(
                "flow {flow_id:?} is already committed"
            )));
        }
        let now = Utc::now();
        activate_periodic_schedule(&mut definition.schedule, now)?;
        validate_definition(&definition).map_err(StoreError::InvalidData)?;
        let queue_order = next_definition_order(&transaction)?;
        let columns = schedule_columns(definition.schedule.as_ref());
        transaction.execute("UPDATE flow_definitions SET committed = 1, queue_order = ?2, committed_at = ?3, graph_revision = 1, schedule_kind = ?4, schedule_at_utc = ?5, daily_time = ?6, schedule_timezone = ?7, period_value = ?8, period_unit = ?9, period_first_at_utc = ?10 WHERE flow_id = ?1", params![flow_id, queue_order, now.to_rfc3339(), columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at])?;
        let first_occurrence = match definition.schedule.as_ref() {
            Some(ScheduleSpec::Once { at }) => Some(*at),
            Some(ScheduleSpec::Periodic { first_at, .. }) => *first_at,
            _ => None,
        };
        if let Some(at) = first_occurrence {
            insert_occurrence(
                &transaction,
                &definition,
                None,
                at,
                OccurrenceState::Pending,
                None,
            )?;
        }
        let result = load_flow(&transaction, flow_id)?;
        advance_definition_state(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn get_flow(&self, flow_id: &str) -> Result<FlowDefinition, StoreError> {
        let connection = self.lock()?;
        load_flow(&connection, flow_id)
    }

    pub fn list_flows(&self, owner: Option<&str>) -> Result<Vec<FlowDefinition>, StoreError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT flow_id FROM flow_definitions WHERE flow_id NOT LIKE 'standalone/%' AND (?1 IS NULL OR owner = ?1) ORDER BY COALESCE(queue_order, 9223372036854775807), created_at, flow_id")?;
        let ids = statement
            .query_map([owner], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.iter().map(|id| load_flow(&connection, id)).collect()
    }

    pub fn freeze_flow(&self, flow_id: &str) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(flow_id))?;
        let definition = load_flow_current(&transaction, flow_id)?;
        if !definition.committed {
            return Err(StoreError::InvalidData(
                "only a committed flow can be frozen".into(),
            ));
        }
        transaction.execute(
            "UPDATE flow_definitions SET frozen = 1 WHERE flow_id = ?1",
            [flow_id],
        )?;
        let result = load_flow(&transaction, flow_id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn disable_flow(&self, flow_id: &str, enabled: bool) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(flow_id))?;
        let previous_enabled = load_flow_base(&transaction, flow_id)?.enabled;
        transaction.execute(
            "UPDATE flow_definitions SET enabled = ?2 WHERE flow_id = ?1",
            params![flow_id, i64::from(enabled)],
        )?;
        if !enabled {
            transaction.execute("UPDATE occurrences SET state = 'SKIPPED', reason = 'DISABLED' WHERE flow_id = ?1 AND state = 'PENDING' AND due_at <= ?2", params![flow_id, Utc::now().to_rfc3339()])?;
        }
        if let Ok(definition) = load_flow_base(&transaction, flow_id) {
            sync_standalone_job_from_flow(&transaction, &definition, false)?;
        }
        let result = load_flow(&transaction, flow_id)?;
        if previous_enabled != enabled && !flow_id.starts_with("standalone/") {
            advance_definition_state(&transaction)?;
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn discard_flow_draft(
        &self,
        flow_id: &str,
        expected_draft_revision: i64,
    ) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(flow_id))?;
        let current = load_flow_base(&transaction, flow_id)?;
        if !current.frozen {
            return Err(StoreError::InvalidData("flow is not frozen".into()));
        }
        if current.draft_revision != expected_draft_revision {
            return Err(StoreError::InvalidData(format!(
                "draft revision conflict: expected {expected_draft_revision}, current {}",
                current.draft_revision
            )));
        }
        transaction.execute(
            "UPDATE flow_definitions SET draft_json = NULL WHERE flow_id = ?1",
            [flow_id],
        )?;
        let result = load_flow(&transaction, flow_id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn unfreeze_flow(
        &self,
        flow_id: &str,
        expected_draft_revision: Option<i64>,
    ) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(flow_id))?;
        let current = load_flow_base(&transaction, flow_id)?;
        if !current.frozen {
            return Err(StoreError::InvalidData("flow is not frozen".into()));
        }
        let draft_json: Option<String> = transaction.query_row(
            "SELECT draft_json FROM flow_definitions WHERE flow_id = ?1",
            [flow_id],
            |row| row.get(0),
        )?;
        let Some(json) = draft_json else {
            if expected_draft_revision.is_some() {
                return Err(StoreError::InvalidData(
                    "no draft exists for the supplied revision".into(),
                ));
            }
            transaction.execute(
                "UPDATE flow_definitions SET frozen = 0 WHERE flow_id = ?1",
                [flow_id],
            )?;
            let result = load_flow(&transaction, flow_id)?;
            transaction.commit()?;
            return Ok(result);
        };
        let expected = expected_draft_revision.ok_or_else(|| {
            StoreError::InvalidData("a draft revision is required when a draft exists".into())
        })?;
        if expected != current.draft_revision {
            return Err(StoreError::InvalidData(format!(
                "draft revision conflict: expected {expected}, current {}",
                current.draft_revision
            )));
        }
        let mut draft: FlowDefinition = serde_json::from_str(&json)?;
        draft.frozen = false;
        let now = Utc::now();
        let schedule_changed = current.schedule != draft.schedule;
        if schedule_changed {
            activate_periodic_schedule(&mut draft.schedule, now)?;
        }
        validate_definition(&draft).map_err(StoreError::InvalidData)?;
        let current_effective_from: Option<String> = transaction.query_row(
            "SELECT schedule_effective_from FROM flow_definitions WHERE flow_id = ?1",
            [flow_id],
            |row| row.get(0),
        )?;
        if schedule_changed && current.mode == ExecutionMode::Scheduled {
            transaction.execute("UPDATE occurrences SET state = 'SUPERSEDED', reason = 'schedule generation replaced' WHERE flow_id = ?1 AND state IN ('PENDING','RESERVED')", [flow_id])?;
        }
        replace_tasks(&transaction, &draft)?;
        let generation = if schedule_changed {
            current.schedule_generation + 1
        } else {
            current.schedule_generation
        };
        let effective_from = if schedule_changed {
            match draft.schedule.as_ref() {
                Some(ScheduleSpec::Daily { timezone, .. }) => {
                    let zone: chrono_tz::Tz = timezone
                        .parse()
                        .map_err(|_| StoreError::InvalidData("unknown schedule timezone".into()))?;
                    Some(
                        now.with_timezone(&zone)
                            .date_naive()
                            .succ_opt()
                            .ok_or_else(|| {
                                StoreError::InvalidData("schedule date overflow".into())
                            })?
                            .to_string(),
                    )
                }
                _ => None,
            }
        } else {
            current_effective_from
        };
        let columns = schedule_columns(draft.schedule.as_ref());
        transaction.execute("UPDATE flow_definitions SET name = ?2, schedule_kind = ?3, schedule_at_utc = ?4, daily_time = ?5, schedule_timezone = ?6, period_value = ?7, period_unit = ?8, period_first_at_utc = ?9, schedule_generation = ?10, schedule_effective_from = ?11, graph_revision = graph_revision + 1, draft_json = NULL, frozen = 0 WHERE flow_id = ?1", params![flow_id, draft.name, columns.kind, columns.at, columns.daily, columns.timezone, columns.period_value, columns.period_unit, columns.period_first_at, generation, effective_from])?;
        let first_occurrence = match draft.schedule.as_ref() {
            Some(ScheduleSpec::Once { at }) => Some(*at),
            Some(ScheduleSpec::Periodic { first_at, .. }) => *first_at,
            _ => None,
        };
        if schedule_changed && let Some(at) = first_occurrence {
            insert_occurrence(
                &transaction,
                &FlowDefinition {
                    schedule_generation: generation,
                    ..draft.clone()
                },
                None,
                at,
                OccurrenceState::Pending,
                None,
            )?;
        }
        sync_standalone_job_from_flow(
            &transaction,
            &FlowDefinition {
                schedule_generation: generation,
                ..draft.clone()
            },
            schedule_changed,
        )?;
        let result = load_flow(&transaction, flow_id)?;
        if !flow_id.starts_with("standalone/") {
            advance_definition_state(&transaction)?;
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn remove_flow_task(
        &self,
        flow_id: &str,
        task_id: &str,
        scope: &str,
        run_id: Option<Uuid>,
        expected_draft_revision: Option<i64>,
    ) -> Result<FlowDefinition, StoreError> {
        if !matches!(scope, "future" | "current" | "both") {
            return Err(StoreError::InvalidData(
                "scope must be future, current, or both".into(),
            ));
        }
        if scope != "future" && run_id.is_none() {
            return Err(StoreError::InvalidData(
                "current and both scopes require --run".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(flow_id))?;
        let current = load_flow_base(&transaction, flow_id)?;
        if !current.committed || !current.frozen {
            return Err(StoreError::InvalidData(
                "flow must be committed and frozen before editing".into(),
            ));
        }
        let edit_future = matches!(scope, "future" | "both");
        let edit_current = matches!(scope, "current" | "both");

        let draft = if edit_future {
            if current.draft_revision > 0 && expected_draft_revision.is_none() {
                return Err(StoreError::InvalidData(
                    "a draft revision is required when a draft exists".into(),
                ));
            }
            let expected = expected_draft_revision.unwrap_or(current.draft_revision);
            if expected != current.draft_revision {
                return Err(StoreError::InvalidData(format!(
                    "draft revision conflict: expected {expected}, current {}",
                    current.draft_revision
                )));
            }
            let draft_json: Option<String> = transaction.query_row(
                "SELECT draft_json FROM flow_definitions WHERE flow_id = ?1",
                [flow_id],
                |row| row.get(0),
            )?;
            let mut draft = draft_json
                .map(|json| serde_json::from_str::<FlowDefinition>(&json))
                .transpose()?
                .unwrap_or_else(|| current.clone());
            remove_future_branch(&mut draft, task_id)?;
            if draft.tasks.is_empty() {
                return Err(StoreError::InvalidData(
                    "future flow cannot be empty".into(),
                ));
            }
            validate_definition(&draft).map_err(StoreError::InvalidData)?;
            Some(draft)
        } else {
            None
        };

        if edit_current {
            let run_id = run_id.expect("run was validated");
            ensure_run_owner(&transaction, flow_id, run_id)?;
            let state = parse_task_state(&transaction.query_row(
                "SELECT state FROM task_runs WHERE run_id = ?1 AND task_id = ?2",
                params![run_id.to_string(), task_id],
                |row| row.get::<_, String>(0),
            )?)?;
            if matches!(
                state,
                TaskRunState::Starting | TaskRunState::Running | TaskRunState::Cancelling
            ) {
                return Err(StoreError::InvalidData(
                    "current task has an active attempt; wait for cleanup".into(),
                ));
            }
            if state.is_terminal() {
                return Err(StoreError::InvalidData(
                    "current task is already terminal".into(),
                ));
            }
            transaction.execute("UPDATE task_runs SET state = 'SKIPPED', cancel_requested = 1 WHERE run_id = ?1 AND task_id = ?2", params![run_id.to_string(), task_id])?;
            transaction.execute("INSERT INTO run_edit_events (run_id, graph_revision, task_id, event, reason, created_at) SELECT ?1, graph_revision, ?2, 'REMOVE', 'SKIPPED_REMOVED', ?3 FROM flow_definitions WHERE flow_id = ?4", params![run_id.to_string(), task_id, Utc::now().to_rfc3339(), flow_id])?;
            propagate_task_states(&transaction, run_id)?;
            let cancel_requested = transaction.query_row(
                "SELECT cancel_requested FROM flow_runs WHERE run_id = ?1",
                [run_id.to_string()],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if let Some(flow_state) =
                aggregate_flow_state(&query_task_runs(&transaction, run_id)?, cancel_requested)
            {
                transaction.execute("UPDATE flow_runs SET state = ?2, finished_at = ?3 WHERE run_id = ?1 AND state NOT IN ('FAILED_TO_START','RECOVERING')", params![run_id.to_string(), flow_state_string(flow_state), Utc::now().to_rfc3339()])?;
            }
        }
        if let Some(draft) = draft {
            save_draft(&transaction, &draft, current.draft_revision + 1)?;
        }
        let result = load_flow(&transaction, flow_id)?;
        transaction.commit()?;
        Ok(result)
    }
}

fn remove_future_branch(definition: &mut FlowDefinition, task_id: &str) -> Result<(), StoreError> {
    if !definition.tasks.iter().any(|task| task.task_id == task_id) {
        return Err(StoreError::InvalidData(format!(
            "task-id {task_id:?} does not exist"
        )));
    }
    let mut removed = BTreeSet::from([task_id.to_owned()]);
    loop {
        let mut newly_removed = Vec::new();
        for task in &definition.tasks {
            if removed.contains(&task.task_id) || task.dependencies.is_empty() {
                continue;
            }
            let removed_edges = task
                .dependencies
                .iter()
                .filter(|edge| removed.contains(&edge.upstream_task_id))
                .count();
            let should_remove = match task.depend_mode {
                DependencyMode::All => removed_edges > 0,
                DependencyMode::Any => removed_edges == task.dependencies.len(),
            };
            if should_remove {
                newly_removed.push(task.task_id.clone());
            }
        }
        if newly_removed.is_empty() {
            break;
        }
        removed.extend(newly_removed);
    }
    definition
        .tasks
        .retain(|task| !removed.contains(&task.task_id));
    for task in &mut definition.tasks {
        task.dependencies
            .retain(|edge| !removed.contains(&edge.upstream_task_id));
    }
    Ok(())
}
