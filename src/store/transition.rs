use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::{Job, JobState};

use super::connection::Store;
use super::error::StoreError;
use super::mapping::{get_job_with, parse_state};

impl Store {
    pub fn request_cancelling(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut connection = self.lock()?;
        self.transition_job(
            &mut connection,
            id,
            "cancel",
            &[JobState::Starting, JobState::Running, JobState::Cancelling],
            |connection| {
                connection.execute(
                    "UPDATE jobs SET state = 'CANCELLING'
                     WHERE id = ?1 AND state IN ('STARTING', 'RUNNING', 'CANCELLING')",
                    [id.to_string()],
                )
            },
        )
    }

    pub fn set_running(&self, id: Uuid, pid: u32) -> Result<Job, StoreError> {
        let mut connection = self.lock()?;
        self.transition_job(
            &mut connection,
            id,
            "start",
            &[JobState::Starting],
            |connection| {
                connection.execute(
                    "UPDATE jobs SET state = 'RUNNING', started_at = ?2, pid = ?3
                     WHERE id = ?1 AND state = 'STARTING'",
                    params![id.to_string(), Utc::now().to_rfc3339(), i64::from(pid)],
                )
            },
        )
    }

    pub fn finish(
        &self,
        id: Uuid,
        exit_code: Option<i32>,
        failure_detail: Option<&str>,
    ) -> Result<Job, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = self.current_state(&transaction, id)?;
        if !matches!(
            current,
            JobState::Starting | JobState::Running | JobState::Cancelling
        ) {
            return Err(StoreError::InvalidTransition {
                id,
                state: current,
                action: "finish",
            });
        }
        let state = if current == JobState::Cancelling {
            JobState::Cancelled
        } else if exit_code == Some(0) {
            JobState::Succeeded
        } else {
            JobState::Failed
        };
        transaction.execute(
            "UPDATE jobs SET state = ?2, finished_at = ?3, exit_code = ?4, failure_detail = ?5
             WHERE id = ?1 AND state IN ('STARTING', 'RUNNING', 'CANCELLING')",
            params![
                id.to_string(),
                state.as_str(),
                Utc::now().to_rfc3339(),
                exit_code,
                failure_detail,
            ],
        )?;
        let daily_schedule: bool = transaction.query_row(
            "SELECT COALESCE(schedule_kind = 'daily', 0) FROM jobs WHERE id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )?;
        if daily_schedule {
            let next_order: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM jobs WHERE state = 'QUEUED'",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                "UPDATE jobs SET state = 'QUEUED', queue_order = ?2 WHERE id = ?1",
                params![id.to_string(), next_order],
            )?;
        }
        let job = get_job_with(&transaction, id)?;
        transaction.commit()?;
        Ok(job)
    }

    pub fn clear_runtime(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = self.current_state(&transaction, id)?;
        if !matches!(
            state,
            JobState::Queued
                | JobState::Succeeded
                | JobState::Failed
                | JobState::Cancelled
                | JobState::Lost
        ) {
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "clear runtime fields",
            });
        }
        transaction.execute("UPDATE jobs SET pid = NULL WHERE id = ?1", [id.to_string()])?;
        let job = get_job_with(&transaction, id)?;
        transaction.commit()?;
        Ok(job)
    }

    pub fn record_failure_detail(&self, id: Uuid, detail: &str) -> Result<Job, StoreError> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE jobs SET failure_detail = ?2 WHERE id = ?1",
            params![id.to_string(), detail],
        )?;
        get_job_with(&connection, id)
    }

    pub fn mark_runtime_jobs_lost(&self) -> Result<(), StoreError> {
        self.mark_runtime_jobs_lost_internal(false).map(|_| ())
    }

    /// Recover runtime rows after a service restart and fence queue claims if
    /// any prior execution may still have survived the crash boundary.
    pub fn recover_runtime_jobs(&self) -> Result<bool, StoreError> {
        self.mark_runtime_jobs_lost_internal(true)
    }

    fn mark_runtime_jobs_lost_internal(&self, fence_queue: bool) -> Result<bool, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE jobs SET state = 'LOST', finished_at = ?1
             WHERE state IN ('STARTING', 'RUNNING', 'CANCELLING')",
            [Utc::now().to_rfc3339()],
        )?;
        let flow_changed = transaction.execute(
            "UPDATE flow_runs SET state = 'RECOVERING'
             WHERE state IN ('STARTING', 'RUNNING', 'CANCELLING')
               AND EXISTS (
                   SELECT 1 FROM attempts
                   WHERE attempts.run_id = flow_runs.run_id
                     AND attempts.state IN ('STARTING', 'RUNNING')
               )",
            [],
        )?;
        transaction.execute(
            "UPDATE attempts SET state = 'LOST', failure_kind = 'RESTART',
             failure_detail = 'service restarted before execution was confirmed stopped',
             finished_at = ?1 WHERE state IN ('STARTING', 'RUNNING')",
            [Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state = 'LOST' WHERE state IN ('STARTING', 'RUNNING', 'CANCELLING')",
            [],
        )?;
        if flow_changed > 0 {
            transaction.execute(
                "UPDATE occurrences SET state = 'UNCERTAIN', reason = 'RESTART_RECOVERY'
                 WHERE occurrence_id IN (SELECT occurrence_id FROM flow_runs WHERE state = 'RECOVERING' AND occurrence_id IS NOT NULL)",
                [],
            )?;
        }
        if fence_queue && (changed > 0 || flow_changed > 0) {
            transaction.execute(
                "UPDATE settings SET queue_locked = 1, recovery_fence = 1 WHERE id = 1",
                [],
            )?;
        }
        transaction.commit()?;
        Ok(changed > 0 || flow_changed > 0)
    }

    /// Resolve one flow left in RECOVERING after a restart.  The explicit
    /// confirmation is the safety boundary that allows the recovery fence to
    /// be lifted without accidentally duplicating a surviving process.
    pub fn reconcile_recovery(
        &self,
        run_id: Uuid,
        confirm_stopped: bool,
    ) -> Result<(), StoreError> {
        if !confirm_stopped {
            return Err(StoreError::InvalidData(
                "--confirm-stopped is required".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: String = transaction.query_row(
            "SELECT state FROM flow_runs WHERE run_id = ?1",
            [run_id.to_string()],
            |row| row.get(0),
        )?;
        if state != "RECOVERING" {
            return Err(StoreError::InvalidData(format!(
                "flow run {run_id} is not recovering"
            )));
        }
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE flow_runs SET state = 'LOST', finished_at = ?2 WHERE run_id = ?1",
            params![run_id.to_string(), now],
        )?;
        transaction.execute("UPDATE occurrences SET state = 'UNCERTAIN', reason = 'RECONCILED_STOPPED' WHERE occurrence_id = (SELECT occurrence_id FROM flow_runs WHERE run_id = ?1) AND state IN ('RESERVED','STARTED')", [run_id.to_string()])?;
        let remaining: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM flow_runs WHERE state = 'RECOVERING'",
            [],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            transaction.execute("UPDATE settings SET recovery_fence = 0 WHERE id = 1", [])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn current_state(
        &self,
        connection: &Connection,
        id: Uuid,
    ) -> Result<JobState, StoreError> {
        let value: Option<String> = connection
            .query_row(
                "SELECT state FROM jobs WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        value
            .ok_or(StoreError::NotFound { id })
            .and_then(|state| parse_state(&state))
    }

    fn transition_job<F>(
        &self,
        connection: &mut Connection,
        id: Uuid,
        action: &'static str,
        allowed: &[JobState],
        update: F,
    ) -> Result<Job, StoreError>
    where
        F: FnOnce(&Connection) -> Result<usize, rusqlite::Error>,
    {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = self.current_state(&transaction, id)?;
        if !allowed.contains(&state) {
            return Err(StoreError::InvalidTransition { id, state, action });
        }
        if update(&transaction)? != 1 {
            return Err(StoreError::InvalidTransition { id, state, action });
        }
        let job = get_job_with(&transaction, id)?;
        transaction.commit()?;
        Ok(job)
    }
}
