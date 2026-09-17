//! Frozen-draft task edits for flow definitions.

use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior};

use crate::domain::flow::{Dependency, DependencyMode, FlowDefinition, FlowRun, FlowTask};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{insert_task, load_flow, load_flow_current, parse_uuid, save_draft};
use super::flows::FlowTaskInput;
use super::standalone::standalone_flow_id;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualRequestStatus {
    pub request_id: uuid::Uuid,
    pub flow_id: String,
    pub run_id: Option<uuid::Uuid>,
    pub result: String,
}

impl Store {
    pub fn add_flow_task(&self, input: FlowTaskInput) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_flow_current(&transaction, &input.flow_id)?;
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
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_flow_current(&transaction, &input.flow_id)?;
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
        if current.committed {
            if current.draft_revision > 0 && expected_draft_revision.is_none() {
                return Err(StoreError::InvalidData(
                    "a draft revision is required when a draft exists".into(),
                ));
            }
            if let Some(expected) = expected_draft_revision
                && expected != current.draft_revision
            {
                return Err(StoreError::InvalidData(format!(
                    "draft revision conflict: expected {expected}, current {}",
                    current.draft_revision
                )));
            }
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
            crate::domain::flow::validate_definition(&draft).map_err(StoreError::InvalidData)?;
            save_draft(&transaction, &draft, current.draft_revision + 1)?;
        } else {
            insert_task(&transaction, &current.flow_id, &task)?;
        }
        let result = load_flow(&transaction, &current.flow_id)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn manual_request(
        &self,
        request_id: uuid::Uuid,
    ) -> Result<ManualRequestStatus, StoreError> {
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
        job_id: uuid::Uuid,
        skip_next: bool,
        request_id: Option<uuid::Uuid>,
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
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
            return Err(StoreError::InvalidData(format!(
                "draft revision conflict: expected {expected}, current {}",
                current.draft_revision
            )));
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
            task.cwd = cwd.to_string_lossy().replace('\\', "/");
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
        crate::domain::flow::validate_definition(&draft).map_err(StoreError::InvalidData)?;
        save_draft(&transaction, &draft, current.draft_revision + 1)?;
        let result = load_flow(&transaction, flow_id)?;
        transaction.commit()?;
        Ok(result)
    }
}
