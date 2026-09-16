//! Flow run and task cancellation state transitions.

use chrono::Utc;
use rusqlite::{TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::flow::{FlowRun, TaskRunState, aggregate_flow_state};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::*;
use super::flow_runtime_mapping::*;

impl Store {
    pub fn cancel_flow_run(&self, flow_id: &str, run_id: Uuid) -> Result<FlowRun, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_run_owner(&transaction, flow_id, run_id)?;
        transaction.execute("UPDATE flow_runs SET cancel_requested = 1, state = CASE WHEN state = 'STARTING' THEN 'CANCELLING' ELSE state END WHERE run_id = ?1 AND state NOT IN ('SUCCEEDED','FAILED','FAILED_TO_START','CANCELLED','SKIPPED','LOST')", [run_id.to_string()])?;
        transaction.execute("UPDATE task_runs SET cancel_requested = 1, state = 'CANCELLED' WHERE run_id = ?1 AND state IN ('WAITING','READY','RETRY_WAIT')", [run_id.to_string()])?;
        transaction.execute("UPDATE task_runs SET state = 'CANCELLING' WHERE run_id = ?1 AND state IN ('STARTING','RUNNING')", [run_id.to_string()])?;
        if let Some(state) = aggregate_flow_state(&query_task_runs(&transaction, run_id)?, true) {
            transaction.execute(
                "UPDATE flow_runs SET state = ?2, finished_at = ?3 WHERE run_id = ?1 AND state NOT IN ('FAILED_TO_START','RECOVERING')",
                params![run_id.to_string(), flow_state_string(state), Utc::now().to_rfc3339()],
            )?;
        }
        let run = load_run(&transaction, run_id)?;
        if run.state.is_terminal() {
            transaction.execute(
                "UPDATE manual_requests SET result = ?2 WHERE run_id = ?1",
                params![run_id.to_string(), flow_state_string(run.state)],
            )?;
            sync_standalone_job_terminal(&transaction, &run.flow_id, flow_state_string(run.state))?;
        }
        transaction.commit()?;
        Ok(run)
    }

    pub fn cancel_all_flow_runs(&self) -> Result<Vec<FlowRun>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = transaction
            .prepare("SELECT run_id FROM flow_runs WHERE state IN ('STARTING','RUNNING','CANCELLING') ORDER BY created_at, run_id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        transaction.execute(
            "UPDATE flow_runs SET cancel_requested = 1, state = CASE WHEN state = 'STARTING' THEN 'CANCELLING' ELSE state END WHERE state IN ('STARTING','RUNNING','CANCELLING')",
            [],
        )?;
        transaction.execute(
            "UPDATE task_runs SET cancel_requested = 1, state = 'CANCELLED' WHERE state IN ('WAITING','READY','RETRY_WAIT') AND run_id IN (SELECT run_id FROM flow_runs WHERE state IN ('CANCELLING','RUNNING'))",
            [],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state = 'CANCELLING' WHERE state IN ('STARTING','RUNNING') AND run_id IN (SELECT run_id FROM flow_runs WHERE state IN ('CANCELLING','RUNNING'))",
            [],
        )?;
        let mut result = Vec::new();
        for id in ids {
            let run_id = parse_uuid(&id)?;
            if let Some(state) = aggregate_flow_state(&query_task_runs(&transaction, run_id)?, true)
            {
                transaction.execute(
                    "UPDATE flow_runs SET state = ?2, finished_at = ?3 WHERE run_id = ?1 AND state NOT IN ('FAILED_TO_START','RECOVERING')",
                    params![id, flow_state_string(state), Utc::now().to_rfc3339()],
                )?;
            }
            let run = load_run(&transaction, run_id)?;
            if run.state.is_terminal() {
                transaction.execute(
                    "UPDATE manual_requests SET result = ?2 WHERE run_id = ?1",
                    params![id, flow_state_string(run.state)],
                )?;
                sync_standalone_job_terminal(
                    &transaction,
                    &run.flow_id,
                    flow_state_string(run.state),
                )?;
            }
            result.push(run);
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn cancel_flow_task(
        &self,
        flow_id: &str,
        task_id: &str,
        run_id: Uuid,
    ) -> Result<FlowRun, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
            transaction.execute(
                "UPDATE task_runs SET cancel_requested = 1, state = 'CANCELLING' WHERE run_id = ?1 AND task_id = ?2",
                params![run_id.to_string(), task_id],
            )?;
            let run = load_run(&transaction, run_id)?;
            transaction.commit()?;
            return Ok(run);
        }
        if state.is_terminal() {
            return Err(StoreError::InvalidData("task is already terminal".into()));
        }
        transaction.execute("UPDATE task_runs SET cancel_requested = 1, state = 'CANCELLED' WHERE run_id = ?1 AND task_id = ?2", params![run_id.to_string(), task_id])?;
        propagate_task_states(&transaction, run_id)?;
        if let Some(state) = aggregate_flow_state(&query_task_runs(&transaction, run_id)?, true) {
            transaction.execute(
                "UPDATE flow_runs SET state = ?2, finished_at = ?3 WHERE run_id = ?1 AND state NOT IN ('FAILED_TO_START','RECOVERING')",
                params![run_id.to_string(), flow_state_string(state), Utc::now().to_rfc3339()],
            )?;
        }
        let run = load_run(&transaction, run_id)?;
        if run.state.is_terminal() {
            transaction.execute(
                "UPDATE manual_requests SET result = ?2 WHERE run_id = ?1",
                params![run_id.to_string(), flow_state_string(run.state)],
            )?;
            sync_standalone_job_terminal(&transaction, &run.flow_id, flow_state_string(run.state))?;
        }
        transaction.commit()?;
        Ok(run)
    }
}
