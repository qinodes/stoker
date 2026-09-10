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
            JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
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
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE jobs SET state = 'LOST', finished_at = ?1
             WHERE state IN ('STARTING', 'RUNNING', 'CANCELLING')",
            [Utc::now().to_rfc3339()],
        )?;
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
