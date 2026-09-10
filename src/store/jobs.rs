use chrono::Utc;
use rusqlite::{TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::{
    Job, JobState, NewJob, normalize_description, validate_description, validate_job_name,
    validate_job_user,
};

use super::connection::Store;
use super::error::StoreError;
use super::mapping::{get_job_with, row_to_job, storage_path};

impl Store {
    pub fn create_job(&self, new_job: NewJob) -> Result<Uuid, StoreError> {
        self.create_job_with_command_line(new_job, None)
    }

    pub fn create_shell_job(
        &self,
        new_job: NewJob,
        command_line: String,
    ) -> Result<Uuid, StoreError> {
        self.create_job_with_command_line(new_job, Some(command_line))
    }

    fn create_job_with_command_line(
        &self,
        new_job: NewJob,
        command_line: Option<String>,
    ) -> Result<Uuid, StoreError> {
        validate_job_name(&new_job.name).map_err(StoreError::InvalidData)?;
        validate_job_user(&new_job.user).map_err(StoreError::InvalidData)?;
        let description = normalize_description(new_job.description);
        validate_description(description.as_deref()).map_err(StoreError::InvalidData)?;
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        let command = serde_json::to_string(&new_job.command)?;
        let cwd = storage_path(&new_job.cwd);
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO jobs
                (id,name,user,cwd,command,command_line,state,created_at,description)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id.to_string(),
                new_job.name,
                new_job.user,
                cwd,
                command,
                command_line,
                JobState::Draft.as_str(),
                created_at.to_rfc3339(),
                description,
            ],
        )?;
        Ok(id)
    }

    pub fn get_job(&self, id: Uuid) -> Result<Job, StoreError> {
        let connection = self.lock()?;
        get_job_with(&connection, id)
    }

    pub fn list_jobs(&self, owner: Option<&str>) -> Result<Vec<Job>, StoreError> {
        self.list_jobs_with_state(owner, None)
    }

    pub fn list_jobs_with_state(
        &self,
        owner: Option<&str>,
        state: Option<JobState>,
    ) -> Result<Vec<Job>, StoreError> {
        let connection = self.lock()?;
        let columns = "id,name,user,cwd,command,command_line,state,queue_order,created_at,
                       committed_at,started_at,finished_at,exit_code,pid,failure_detail,
                       description,description_revision";
        let order = "ORDER BY CASE state WHEN 'RUNNING' THEN 0 WHEN 'QUEUED' THEN 1 ELSE 2 END,
                             CASE WHEN state = 'QUEUED' THEN queue_order END,
                             CASE WHEN state <> 'QUEUED' THEN COALESCE(committed_at, created_at) END DESC,
                             id";
        let query = match (owner.is_some(), state) {
            (true, Some(_)) => {
                format!("SELECT {columns} FROM jobs WHERE user = ?1 AND state = ?2 {order}")
            }
            (false, Some(_)) => format!("SELECT {columns} FROM jobs WHERE state = ?1 {order}"),
            (true, None) => format!("SELECT {columns} FROM jobs WHERE user = ?1 {order}"),
            (false, None) => format!("SELECT {columns} FROM jobs {order}"),
        };
        let mut statement = connection.prepare(&query)?;
        let rows = match (owner, state) {
            (Some(owner), Some(state)) => statement
                .query_map(params![owner, state.as_str()], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(state)) => statement
                .query_map([state.as_str()], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(owner), None) => statement
                .query_map([owner], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => statement
                .query_map([], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    /// Delete terminal rows atomically and return them so callers can remove
    /// their filesystem artifacts after the database commit.
    pub fn clean_terminal_jobs(&self) -> Result<Vec<Job>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let jobs = {
            let mut statement = transaction.prepare(
                "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,failure_detail,
                        description,description_revision
                 FROM jobs
                 WHERE state IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'LOST')",
            )?;
            statement
                .query_map([], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?
        };
        for job in &jobs {
            transaction.execute("DELETE FROM jobs WHERE id = ?1", [job.id.to_string()])?;
        }
        transaction.commit()?;
        Ok(jobs)
    }
}
