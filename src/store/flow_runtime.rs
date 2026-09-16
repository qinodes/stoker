//! Flow run claiming, attempt state, and run history persistence.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::flow::{
    AttemptState, DependencyEvaluation, FlowDefinition, FlowRun, FlowTask, Occurrence, TaskRun,
    TaskRunState, aggregate_flow_state, evaluate_dependencies, validate_definition,
};
use crate::store::{FlowAttemptResult, FlowTaskExecution};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::*;
use super::flow_runtime_mapping::*;

impl Store {
    pub fn create_flow_run(
        &self,
        flow_id: &str,
        source: &str,
        skip_next: bool,
        request_id: Option<Uuid>,
    ) -> Result<FlowRun, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request_hash = serde_json::to_string(&(flow_id, source, skip_next))?;
        if let Some(request_id) = request_id
            && let Some((old_flow, old_hash, old_run)) = transaction.query_row("SELECT flow_id, parameters_hash, run_id FROM manual_requests WHERE request_id = ?1", [request_id.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))).optional()?
        {
            if old_flow != flow_id || old_hash != request_hash {
                return Err(StoreError::InvalidData(
                    "request-id was already used with different parameters".into(),
                ));
            }
            let old_run = old_run
                .ok_or_else(|| StoreError::InvalidData("request has no run".into()))?;
            return load_run(&transaction, parse_uuid(&old_run)?);
        }
        let definition = load_flow_base(&transaction, flow_id)?;
        validate_definition(&definition).map_err(StoreError::InvalidData)?;
        if !definition.committed {
            return Err(StoreError::InvalidData(
                "flow must be committed before it can run".into(),
            ));
        }
        if definition.frozen {
            return Err(StoreError::InvalidData("flow is frozen".into()));
        }
        if queue_locked_with(&transaction)? {
            return Err(StoreError::QueueLocked);
        }
        let workspace_mode = parse_mode(&transaction.query_row(
            "SELECT mode FROM settings WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )?)?;
        if definition.mode != workspace_mode {
            return Err(StoreError::InvalidData(
                "flow mode is not the current workspace mode".into(),
            ));
        }
        if fence_with(&transaction)? {
            return Err(StoreError::InvalidData(
                "queue recovery fence is active".into(),
            ));
        }
        let (active_attempts, max_concurrency): (i64, i64) = transaction.query_row(
            "SELECT
                 (SELECT COUNT(*) FROM attempts WHERE state IN ('STARTING','RUNNING')),
                 max_concurrency
             FROM settings WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if definition.mode == crate::domain::flow::ExecutionMode::Scheduled
            && active_attempts >= max_concurrency
        {
            return Err(StoreError::InvalidData(
                "scheduled execution capacity is full".into(),
            ));
        }
        if has_active_run(&transaction, flow_id)? {
            return Err(StoreError::InvalidData(
                "flow already has an active run".into(),
            ));
        }
        let occurrence_id = if skip_next {
            if definition.schedule.as_ref().is_some_and(|schedule| {
                matches!(schedule, crate::domain::flow::ScheduleSpec::Daily { .. })
            }) {
                ensure_next_daily_occurrence(&transaction, &definition, Utc::now())?;
            }
            Some(reserve_next_occurrence(&transaction, &definition)?)
        } else {
            None
        };
        let run_id = Uuid::new_v4();
        let now = Utc::now();
        transaction.execute("INSERT INTO flow_runs (run_id, flow_id, internal_definition_id, schedule_generation, source, state, occurrence_id, definition_snapshot, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'STARTING', ?6, ?7, ?8)", params![run_id.to_string(), flow_id, definition.internal_id.to_string(), definition.schedule_generation, source, occurrence_id.map(|id| id.to_string()), serde_json::to_string(&definition)?, now.to_rfc3339()])?;
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
        if let Some(request_id) = request_id {
            transaction.execute("INSERT INTO manual_requests (request_id, flow_id, parameters_hash, run_id, result, created_at) VALUES (?1, ?2, ?3, ?4, 'CREATED', ?5)", params![request_id.to_string(), flow_id, request_hash, run_id.to_string(), now.to_rfc3339()])?;
        }
        let run = load_run(&transaction, run_id)?;
        transaction.commit()?;
        Ok(run)
    }

    pub fn get_flow_run(&self, run_id: Uuid) -> Result<FlowRun, StoreError> {
        let connection = self.lock()?;
        load_run(&connection, run_id)
    }

    pub fn list_flow_runs(&self, flow_id: &str) -> Result<Vec<FlowRun>, StoreError> {
        let connection = self.lock()?;
        let ids = connection
            .prepare("SELECT run_id FROM flow_runs WHERE flow_id = ?1 ORDER BY created_at, run_id")?
            .query_map([flow_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| load_run(&connection, parse_uuid(&id)?))
            .collect()
    }

    pub fn list_occurrences(&self, flow_id: &str) -> Result<Vec<Occurrence>, StoreError> {
        let connection = self.lock()?;
        Ok(connection.prepare("SELECT occurrence_id, flow_id, schedule_generation, local_date, due_at, state, reason FROM occurrences WHERE flow_id = ?1 ORDER BY due_at, occurrence_id")?.query_map([flow_id], occurrence_from_row)?.collect::<Result<Vec<_>, _>>()?)
    }

    /// Atomically materialise due daily occurrences, claim one eligible task,
    /// and create its attempt.  A task claim is not a flow start: the flow
    /// remains STARTING until the first root process is acknowledged.
    pub fn claim_flow_task(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Option<FlowTaskExecution>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if queue_locked_with(&transaction)? {
            materialize_daily_occurrences(&transaction, now)?;
            expire_occurrences(&transaction, now)?;
            settle_daily_occurrences(&transaction, now, false, Some("SKIPPED_LOCKED"))?;
            transaction.commit()?;
            return Ok(None);
        }
        if fence_with(&transaction)? {
            materialize_daily_occurrences(&transaction, now)?;
            expire_occurrences(&transaction, now)?;
            settle_daily_occurrences(&transaction, now, false, Some("SKIPPED_RECOVERY"))?;
            transaction.commit()?;
            return Ok(None);
        }
        materialize_daily_occurrences(&transaction, now)?;
        expire_occurrences(&transaction, now)?;
        let mode = parse_mode(&transaction.query_row(
            "SELECT mode FROM settings WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )?)?;
        if mode == crate::domain::flow::ExecutionMode::Serial {
            start_serial_standalone_run(&transaction)?;
        }
        start_due_flow_run(&transaction, now, mode)?;
        settle_daily_occurrences(&transaction, now, false, None)?;
        let mut run_rows = transaction.prepare("SELECT run_id, flow_id, source, state, definition_snapshot FROM flow_runs WHERE state IN ('STARTING','RUNNING') ORDER BY created_at, run_id")?.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)))?.collect::<Result<Vec<_>, _>>()?;
        let mut candidates: Vec<(Uuid, String, FlowTask, TaskRun, bool)> = Vec::new();
        for (run_id_text, flow_id, run_source, run_state, snapshot) in run_rows.drain(..) {
            let run_id = parse_uuid(&run_id_text)?;
            let definition: FlowDefinition = serde_json::from_str(&snapshot)?;
            let current_definition = load_flow_base(&transaction, &flow_id)?;
            if current_definition.mode != mode
                || current_definition.frozen
                || run_source == "AUTOMATIC" && !current_definition.enabled
            {
                continue;
            }
            let task_runs = query_task_runs(&transaction, run_id)?;
            if run_state == "STARTING"
                && task_runs.iter().any(|task| {
                    matches!(task.state, TaskRunState::Starting | TaskRunState::Running)
                })
            {
                continue;
            }
            let outcomes = task_outcomes(&task_runs);
            let mut starting_candidate_added = false;
            for task in definition.tasks {
                let Some(task_run) = task_runs.iter().find(|item| item.task_id == task.task_id)
                else {
                    continue;
                };
                if task_run.state == TaskRunState::Waiting {
                    match evaluate_dependencies(&task, &outcomes) {
                        DependencyEvaluation::Ready => {
                            transaction.execute("UPDATE task_runs SET state = 'READY' WHERE run_id = ?1 AND task_id = ?2", params![run_id_text, task.task_id])?;
                        }
                        DependencyEvaluation::Skipped => {
                            transaction.execute("UPDATE task_runs SET state = 'SKIPPED' WHERE run_id = ?1 AND task_id = ?2", params![run_id_text, task.task_id])?;
                        }
                        DependencyEvaluation::Waiting => {}
                    }
                }
                let current = query_task_runs(&transaction, run_id)?
                    .into_iter()
                    .find(|item| item.task_id == task.task_id)
                    .expect("task run exists");
                let retry_due = current.state == TaskRunState::RetryWait
                    && current.next_attempt_at.is_some_and(|at| at <= now);
                let ready = current.state == TaskRunState::Ready || retry_due;
                if !ready || run_state == "STARTING" && !task.dependencies.is_empty() {
                    continue;
                }
                if run_state == "STARTING" && starting_candidate_added {
                    continue;
                }
                candidates.push((
                    run_id,
                    flow_id.clone(),
                    task,
                    current,
                    run_state == "STARTING",
                ));
                if run_state == "STARTING" {
                    starting_candidate_added = true;
                }
            }
        }
        candidates.sort_by_key(|(run_id, flow_id, task, task_run, _)| {
            (
                task_run.next_attempt_at.unwrap_or(now),
                task.sequence,
                flow_id.clone(),
                *run_id,
            )
        });
        let Some((run_id, flow_id, task, current, first_root)) = candidates.into_iter().next()
        else {
            transaction.commit()?;
            return Ok(None);
        };
        let attempt_number = current.attempt_count + 1;
        let attempt_id = Uuid::new_v4();
        transaction.execute("UPDATE task_runs SET state = 'STARTING', attempt_count = ?3, next_attempt_at = NULL WHERE run_id = ?1 AND task_id = ?2", params![run_id.to_string(), task.task_id, i64::from(attempt_number)])?;
        transaction.execute("INSERT INTO attempts (attempt_id, run_id, task_id, number, state, started_at) VALUES (?1, ?2, ?3, ?4, 'STARTING', ?5)", params![attempt_id.to_string(), run_id.to_string(), task.task_id, i64::from(attempt_number), now.to_rfc3339()])?;
        transaction.commit()?;
        Ok(Some(FlowTaskExecution {
            run_id,
            flow_id,
            task_id: task.task_id.clone(),
            attempt_id,
            attempt_number,
            first_root,
            task,
        }))
    }

    pub fn settle_daily_occurrences(
        &self,
        now: DateTime<Utc>,
        capacity_full: bool,
    ) -> Result<(), StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        settle_daily_occurrences(&transaction, now, capacity_full, None)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn mark_flow_attempt_running(&self, attempt_id: Uuid) -> Result<(), StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run_id, task_id, flow_id, source): (String, String, String, String) = transaction.query_row(
            "SELECT a.run_id, a.task_id, r.flow_id, r.source FROM attempts a JOIN flow_runs r ON r.run_id = a.run_id WHERE a.attempt_id = ?1",
            [attempt_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        transaction.execute(
            "UPDATE attempts SET state = 'RUNNING' WHERE attempt_id = ?1 AND state = 'STARTING'",
            [attempt_id.to_string()],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state = 'RUNNING' WHERE run_id = ?1 AND task_id = ?2 AND state = 'STARTING'",
            params![run_id, task_id],
        )?;
        transaction.execute(
            "UPDATE flow_runs SET state = 'RUNNING', started_at = COALESCE(started_at, ?2) WHERE run_id = ?1 AND state = 'STARTING'",
            params![run_id, Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "UPDATE occurrences SET state = CASE WHEN state = 'RESERVED' THEN CASE WHEN ?2 = 'MANUAL' THEN 'REPLACED' ELSE 'STARTED' END ELSE state END WHERE occurrence_id = (SELECT occurrence_id FROM flow_runs WHERE run_id = ?1)",
            params![run_id, source],
        )?;
        sync_standalone_job_running(&transaction, &flow_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn flow_attempt_cancel_requested(&self, attempt_id: Uuid) -> Result<bool, StoreError> {
        let connection = self.lock()?;
        Ok(connection.query_row(
            "SELECT (a.task_id IS NOT NULL AND (t.cancel_requested = 1 OR r.cancel_requested = 1)) FROM attempts a JOIN task_runs t ON t.run_id = a.run_id AND t.task_id = a.task_id JOIN flow_runs r ON r.run_id = a.run_id WHERE a.attempt_id = ?1",
            [attempt_id.to_string()],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    pub fn finish_flow_attempt(
        &self,
        attempt_id: Uuid,
        result: FlowAttemptResult,
    ) -> Result<FlowRun, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run_id_text, task_id, number, run_state, run_source): (
            String,
            String,
            i64,
            String,
            String,
        ) = transaction.query_row("SELECT a.run_id, a.task_id, a.number, r.state, r.source FROM attempts a JOIN flow_runs r ON r.run_id = a.run_id WHERE a.attempt_id = ?1", [attempt_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?;
        let run_id = parse_uuid(&run_id_text)?;
        let now = Utc::now();
        let (state, exit_code, kind, detail) = match &result {
            FlowAttemptResult::Succeeded { exit_code } => {
                (AttemptState::Succeeded, Some(*exit_code), None, None)
            }
            FlowAttemptResult::Failed {
                exit_code,
                kind,
                detail,
            } => (
                AttemptState::Failed,
                *exit_code,
                Some(kind.clone()),
                Some(detail.clone()),
            ),
            FlowAttemptResult::Cancelled { detail } => (
                AttemptState::Cancelled,
                None,
                Some("CANCELLED".into()),
                Some(detail.clone()),
            ),
            FlowAttemptResult::Lost { detail } => (
                AttemptState::Lost,
                None,
                Some("LOST".into()),
                Some(detail.clone()),
            ),
        };
        transaction.execute("UPDATE attempts SET state = ?2, exit_code = ?3, failure_kind = ?4, failure_detail = ?5, finished_at = ?6 WHERE attempt_id = ?1", params![attempt_id.to_string(), attempt_state_string(state), exit_code, kind, detail, now.to_rfc3339()])?;
        let snapshot: String = transaction.query_row(
            "SELECT definition_snapshot FROM flow_runs WHERE run_id = ?1",
            [&run_id_text],
            |row| row.get(0),
        )?;
        let definition: FlowDefinition = serde_json::from_str(&snapshot)?;
        let task_definition = definition
            .tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| {
                StoreError::InvalidData(format!("task {task_id:?} missing from run snapshot"))
            })?;
        let start_failed = matches!(
            &result,
            FlowAttemptResult::Failed { kind, .. }
                if matches!(kind.as_str(), "STARTUP" | "STARTUP_TIMEOUT" | "SPAWN")
        );
        let retry = matches!(result, FlowAttemptResult::Failed { .. })
            && !start_failed
            && number <= i64::from(task_definition.retry);
        if retry {
            transaction.execute("UPDATE task_runs SET state = 'RETRY_WAIT', next_attempt_at = ?3 WHERE run_id = ?1 AND task_id = ?2", params![run_id_text, task_id, (now + crate::domain::flow::RETRY_DELAY).to_rfc3339()])?;
        } else {
            let task_state = match result {
                FlowAttemptResult::Succeeded { .. } => TaskRunState::Succeeded,
                FlowAttemptResult::Failed { .. } => TaskRunState::Failed,
                FlowAttemptResult::Cancelled { .. } => TaskRunState::Cancelled,
                FlowAttemptResult::Lost { .. } => TaskRunState::Lost,
            };
            transaction.execute("UPDATE task_runs SET state = ?3, next_attempt_at = NULL WHERE run_id = ?1 AND task_id = ?2", params![run_id_text, task_id, task_state_string(task_state)])?;
        }
        let uncertain = matches!(&result, FlowAttemptResult::Lost { .. });
        if uncertain {
            transaction.execute(
                "UPDATE flow_runs SET state = 'RECOVERING' WHERE run_id = ?1",
                [&run_id_text],
            )?;
            transaction.execute(
                "UPDATE occurrences SET state = 'UNCERTAIN', reason = 'PROCESS_RESULT_UNKNOWN' WHERE occurrence_id = (SELECT occurrence_id FROM flow_runs WHERE run_id = ?1) AND state IN ('RESERVED','STARTED')",
                [&run_id_text],
            )?;
            transaction.execute(
                "UPDATE settings SET queue_locked = 1, recovery_fence = 1 WHERE id = 1",
                [],
            )?;
        } else if run_state == "STARTING" && !retry {
            if !start_failed {
                transaction.execute(
                    "UPDATE flow_runs SET state = 'RUNNING', started_at = ?2 WHERE run_id = ?1",
                    params![run_id_text, now.to_rfc3339()],
                )?;
                transaction.execute("UPDATE occurrences SET state = CASE WHEN state = 'RESERVED' THEN CASE WHEN ?2 = 'MANUAL' THEN 'REPLACED' ELSE 'STARTED' END ELSE state END WHERE occurrence_id = (SELECT occurrence_id FROM flow_runs WHERE run_id = ?1)", params![run_id_text, run_source])?;
                propagate_task_states(&transaction, run_id)?;
                if let Some(state) =
                    aggregate_flow_state(&query_task_runs(&transaction, run_id)?, false)
                {
                    transaction.execute(
                        "UPDATE flow_runs SET state = ?2, finished_at = ?3 WHERE run_id = ?1",
                        params![run_id_text, flow_state_string(state), now.to_rfc3339()],
                    )?;
                }
            } else {
                transaction.execute("UPDATE flow_runs SET state = 'FAILED_TO_START', finished_at = ?2 WHERE run_id = ?1", params![run_id_text, now.to_rfc3339()])?;
                transaction.execute("UPDATE task_runs SET state = 'SKIPPED' WHERE run_id = ?1 AND state NOT IN ('SUCCEEDED','FAILED','CANCELLED','SKIPPED','LOST')", [&run_id_text])?;
                if run_source == "MANUAL" {
                    transaction.execute("UPDATE occurrences SET state = 'PENDING', reason = NULL WHERE occurrence_id = (SELECT occurrence_id FROM flow_runs WHERE run_id = ?1) AND state = 'RESERVED'", [&run_id_text])?;
                } else {
                    transaction.execute("UPDATE occurrences SET state = 'SKIPPED', reason = 'SKIPPED_START_FAILED' WHERE occurrence_id = (SELECT occurrence_id FROM flow_runs WHERE run_id = ?1)", [&run_id_text])?;
                }
            }
        } else if !retry {
            propagate_task_states(&transaction, run_id)?;
            if let Some(state) = aggregate_flow_state(
                &query_task_runs(&transaction, run_id)?,
                transaction.query_row(
                    "SELECT cancel_requested FROM flow_runs WHERE run_id = ?1",
                    [&run_id_text],
                    |row| row.get::<_, i64>(0),
                )? != 0,
            ) {
                transaction.execute("UPDATE flow_runs SET state = ?2, finished_at = ?3 WHERE run_id = ?1 AND state NOT IN ('FAILED_TO_START','RECOVERING')", params![run_id_text, flow_state_string(state), now.to_rfc3339()])?;
            }
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

    pub fn flow_active_count(&self) -> Result<usize, StoreError> {
        let connection = self.lock()?;
        Ok(connection.query_row("SELECT COUNT(*) FROM flow_runs WHERE state IN ('STARTING','RUNNING','CANCELLING','RECOVERING')", [], |row| row.get::<_, i64>(0))? as usize)
    }
}
