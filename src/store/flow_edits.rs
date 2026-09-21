//! Frozen-draft task edits for flow definitions.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use rusqlite::{TransactionBehavior, params};

use crate::config::command_cwd;
use crate::domain::flow::{
    Dependency, DependencyMode, FlowDefinition, FlowTask, TaskRunState, aggregate_flow_state,
    validate_definition, validate_task_metadata,
};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{
    flow_state_string, insert_task, load_flow, load_flow_base, load_flow_current, parse_task_state,
    propagate_task_states, query_task_runs, save_draft,
};
use super::flow_runtime_mapping::ensure_run_owner;
use super::flow_sources::require_manual_source;
use super::flows::FlowTaskInput;

impl Store {
    pub fn add_flow_task(&self, input: FlowTaskInput) -> Result<FlowDefinition, StoreError> {
        validate_task_metadata(&input.task_id, &input.name).map_err(StoreError::InvalidData)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_manual_source(&transaction, Some(&input.flow_id))?;
        // A frozen definition may already have task or schedule changes in its
        // future draft. Add the new task to that draft instead of rebuilding
        // from the committed graph and silently discarding earlier edits.
        let current = load_flow(&transaction, &input.flow_id)?;
        if current.committed && !current.frozen {
            return Err(StoreError::InvalidData(
                "committed flow must be frozen before editing".into(),
            ));
        }
        if current
            .tasks
            .iter()
            .any(|task| task.task_id == input.task_id)
        {
            return Err(StoreError::InvalidData(format!(
                "task-id {:?} already exists in flow",
                input.task_id
            )));
        }
        let task = FlowTask {
            task_id: input.task_id,
            name: input.name,
            cwd: input.cwd,
            command: input.command,
            retry: input.retry,
            dependencies: input.dependencies,
            depend_mode: input.depend_mode,
            sequence: current.tasks.len() as u64,
        };
        if current.committed {
            let mut draft = current.clone();
            draft.tasks.push(task);
            save_draft(&transaction, &draft, current.draft_revision + 1)?;
        } else {
            insert_task(&transaction, &current.flow_id, &task)?;
        }
        let result = load_flow(&transaction, &current.flow_id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn add_flow_task_with_revision(
        &self,
        input: FlowTaskInput,
        expected_draft_revision: Option<i64>,
    ) -> Result<FlowDefinition, StoreError> {
        self.add_flow_task_with_revision_inner(input, expected_draft_revision, false)
    }

    pub fn add_scheduled_flow_task(
        &self,
        input: FlowTaskInput,
        expected_draft_revision: i64,
    ) -> Result<FlowDefinition, StoreError> {
        self.add_flow_task_with_revision_inner(input, Some(expected_draft_revision), true)
    }

    fn add_flow_task_with_revision_inner(
        &self,
        input: FlowTaskInput,
        expected_draft_revision: Option<i64>,
        require_scheduled: bool,
    ) -> Result<FlowDefinition, StoreError> {
        validate_task_metadata(&input.task_id, &input.name).map_err(StoreError::InvalidData)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if require_scheduled {
            super::flow_mapping::require_scheduled_mode(&transaction)?;
        }
        require_manual_source(&transaction, Some(&input.flow_id))?;
        // Keep every edit already present in draft_json. load_flow_current
        // intentionally returns the committed graph for apply operations, so
        // using it here would overwrite earlier draft changes.
        let current = load_flow(&transaction, &input.flow_id)?;
        if current.committed && !current.frozen {
            return Err(StoreError::InvalidData(
                "committed flow must be frozen before editing".into(),
            ));
        }
        if current
            .tasks
            .iter()
            .any(|task| task.task_id == input.task_id)
        {
            return Err(StoreError::InvalidData(format!(
                "task-id {:?} already exists in flow",
                input.task_id
            )));
        }
        if current.draft_revision > 0 && expected_draft_revision.is_none() {
            return Err(StoreError::InvalidData(
                "a draft revision is required when a draft exists".into(),
            ));
        }
        if let Some(expected) = expected_draft_revision
            && expected != current.draft_revision
        {
            return Err(StoreError::DraftRevisionConflict {
                expected,
                current: current.draft_revision,
            });
        }
        let task = crate::domain::flow::FlowTask {
            task_id: input.task_id,
            name: input.name,
            cwd: input.cwd,
            command: input.command,
            retry: input.retry,
            dependencies: input.dependencies,
            depend_mode: input.depend_mode,
            sequence: current.tasks.len() as u64,
        };
        if current.committed {
            let mut draft = current.clone();
            draft.tasks.push(task);
            validate_definition(&draft).map_err(StoreError::InvalidData)?;
            save_draft(&transaction, &draft, current.draft_revision + 1)?;
        } else {
            insert_task(&transaction, &current.flow_id, &task)?;
        }
        let result = load_flow(&transaction, &current.flow_id)?;
        transaction.commit()?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_flow_task_draft(
        &self,
        flow_id: &str,
        task_id: &str,
        command: Option<&str>,
        cwd: Option<&Path>,
        retry: Option<u32>,
        dependencies: Option<Vec<Dependency>>,
        depend_mode: Option<DependencyMode>,
        expected_draft_revision: Option<i64>,
    ) -> Result<FlowDefinition, StoreError> {
        self.set_flow_task_draft_inner(
            flow_id,
            task_id,
            command,
            cwd,
            retry,
            dependencies,
            depend_mode,
            expected_draft_revision,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_scheduled_flow_task_draft(
        &self,
        flow_id: &str,
        task_id: &str,
        command: Option<&str>,
        cwd: Option<&Path>,
        retry: Option<u32>,
        dependencies: Option<Vec<Dependency>>,
        depend_mode: Option<DependencyMode>,
        expected_draft_revision: i64,
    ) -> Result<FlowDefinition, StoreError> {
        self.set_flow_task_draft_inner(
            flow_id,
            task_id,
            command,
            cwd,
            retry,
            dependencies,
            depend_mode,
            Some(expected_draft_revision),
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn set_flow_task_draft_inner(
        &self,
        flow_id: &str,
        task_id: &str,
        command: Option<&str>,
        cwd: Option<&Path>,
        retry: Option<u32>,
        dependencies: Option<Vec<Dependency>>,
        depend_mode: Option<DependencyMode>,
        expected_draft_revision: Option<i64>,
        require_scheduled: bool,
    ) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if require_scheduled {
            super::flow_mapping::require_scheduled_mode(&transaction)?;
        }
        require_manual_source(&transaction, Some(flow_id))?;
        let current = load_flow_current(&transaction, flow_id)?;
        if !current.committed || !current.frozen {
            return Err(StoreError::InvalidData(
                "flow must be committed and frozen before editing a task".into(),
            ));
        }
        if current.draft_revision > 0 && expected_draft_revision.is_none() {
            return Err(StoreError::InvalidData(
                "a draft revision is required when a draft exists".into(),
            ));
        }
        if let Some(expected) = expected_draft_revision
            && expected != current.draft_revision
        {
            return Err(StoreError::DraftRevisionConflict {
                expected,
                current: current.draft_revision,
            });
        }
        let mut draft = transaction
            .query_row(
                "SELECT draft_json FROM flow_definitions WHERE flow_id = ?1",
                [flow_id],
                |row| row.get::<_, Option<String>>(0),
            )?
            .map(|json| serde_json::from_str::<FlowDefinition>(&json))
            .transpose()?
            .unwrap_or_else(|| current.clone());
        let task = draft
            .tasks
            .iter_mut()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| {
                StoreError::InvalidData(format!(
                    "task {task_id:?} does not exist in flow {flow_id:?}"
                ))
            })?;
        if let Some(command) = command {
            task.command = command.to_owned();
        }
        if let Some(cwd) = cwd {
            task.cwd = command_cwd(cwd);
        }
        if let Some(retry) = retry {
            task.retry = retry;
        }
        if let Some(dependencies) = dependencies {
            task.dependencies = dependencies;
        }
        if let Some(depend_mode) = depend_mode {
            task.depend_mode = depend_mode;
        }
        validate_definition(&draft).map_err(StoreError::InvalidData)?;
        save_draft(&transaction, &draft, current.draft_revision + 1)?;
        let result = load_flow(&transaction, flow_id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn remove_flow_task(
        &self,
        flow_id: &str,
        task_id: &str,
        scope: &str,
        run_id: Option<uuid::Uuid>,
        expected_draft_revision: Option<i64>,
    ) -> Result<FlowDefinition, StoreError> {
        self.remove_flow_task_inner(
            flow_id,
            task_id,
            scope,
            run_id,
            expected_draft_revision,
            false,
        )
    }

    pub fn remove_scheduled_flow_task(
        &self,
        flow_id: &str,
        task_id: &str,
        expected_draft_revision: i64,
    ) -> Result<FlowDefinition, StoreError> {
        self.remove_flow_task_inner(
            flow_id,
            task_id,
            "future",
            None,
            Some(expected_draft_revision),
            true,
        )
    }

    fn remove_flow_task_inner(
        &self,
        flow_id: &str,
        task_id: &str,
        scope: &str,
        run_id: Option<uuid::Uuid>,
        expected_draft_revision: Option<i64>,
        require_scheduled: bool,
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
        if require_scheduled {
            super::flow_mapping::require_scheduled_mode(&transaction)?;
        }
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
                return Err(StoreError::DraftRevisionConflict {
                    expected,
                    current: current.draft_revision,
                });
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
