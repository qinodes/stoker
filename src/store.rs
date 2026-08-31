use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{Job, JobState, NewJob};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    user TEXT NOT NULL,
    repository TEXT NOT NULL,
    git_commit TEXT NOT NULL,
    cwd TEXT NOT NULL,
    command TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    committed_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    exit_code INTEGER,
    pid INTEGER,
    execution_dir TEXT,
    failure_detail TEXT
);
CREATE INDEX IF NOT EXISTS jobs_state_commit_id
    ON jobs (state, committed_at, id);
"#;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("job {id} does not exist")]
    NotFound { id: Uuid },
    #[error("cannot {action} job {id} while it is {state}")]
    InvalidTransition {
        id: Uuid,
        state: JobState,
        action: &'static str,
    },
    #[error("store lock is poisoned")]
    Poisoned,
    #[error("invalid value in jobs table: {0}")]
    InvalidData(String),
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|err| StoreError::InvalidData(err.to_string()))?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn create_job(&self, new_job: NewJob) -> Result<Uuid, StoreError> {
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        let command = serde_json::to_string(&new_job.command)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO jobs (id,name,user,repository,git_commit,cwd,command,state,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id.to_string(),
                new_job.name,
                new_job.user,
                new_job.repository.to_string_lossy().as_ref(),
                new_job.git_commit,
                new_job.cwd.to_string_lossy().as_ref(),
                command,
                JobState::Draft.as_str(),
                created_at.to_rfc3339(),
            ],
        )?;
        Ok(id)
    }

    pub fn get_job(&self, id: Uuid) -> Result<Job, StoreError> {
        let conn = self.lock()?;
        get_job_with(&conn, id)
    }

    pub fn list_jobs(&self, owner: Option<&str>) -> Result<Vec<Job>, StoreError> {
        let conn = self.lock()?;
        let mut statement = if owner.is_some() {
            conn.prepare(
                "SELECT id,name,user,repository,git_commit,cwd,command,state,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,execution_dir,failure_detail
                 FROM jobs WHERE user = ?1
                 ORDER BY COALESCE(committed_at, created_at), created_at, id",
            )?
        } else {
            conn.prepare(
                "SELECT id,name,user,repository,git_commit,cwd,command,state,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,execution_dir,failure_detail
                 FROM jobs
                 ORDER BY COALESCE(committed_at, created_at), created_at, id",
            )?
        };
        let rows = if let Some(owner) = owner {
            statement
                .query_map([owner], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn commit_job(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        self.transition_job(&mut conn, id, "commit", &[JobState::Draft], |conn| {
            conn.execute(
                "UPDATE jobs SET state = 'QUEUED', committed_at = ?2 WHERE id = ?1 AND state = 'DRAFT'",
                params![id.to_string(), Utc::now().to_rfc3339()],
            )
        })
    }

    pub fn cancel_not_started(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        self.transition_job(
            &mut conn,
            id,
            "cancel",
            &[JobState::Draft, JobState::Queued],
            |conn| {
                conn.execute(
                    "UPDATE jobs SET state = 'CANCELLED', finished_at = ?2
                     WHERE id = ?1 AND state IN ('DRAFT', 'QUEUED')",
                    params![id.to_string(), Utc::now().to_rfc3339()],
                )
            },
        )
    }

    /// Transition an active execution into the cancellation phase. The
    /// scheduler owns terminating the process and completing the terminal
    /// transition; this durable marker prevents normal completion from
    /// reporting success after a cancellation request.
    pub fn request_cancelling(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        self.transition_job(
            &mut conn,
            id,
            "cancel",
            &[JobState::Starting, JobState::Running, JobState::Cancelling],
            |conn| {
                conn.execute(
                    "UPDATE jobs SET state = 'CANCELLING'
                     WHERE id = ?1 AND state IN ('STARTING', 'RUNNING', 'CANCELLING')",
                    [id.to_string()],
                )
            },
        )
    }

    /// Atomically claim the oldest queued job. SQLite's write transaction keeps
    /// concurrent scheduler instances from selecting the same job.
    pub fn claim_next(&self) -> Result<Option<Job>, StoreError> {
        let conn = self.lock()?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let selected: Option<String> = conn
                .query_row(
                    "SELECT id FROM jobs WHERE state = 'QUEUED'
                     ORDER BY committed_at, id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(id_text) = selected else {
                conn.execute_batch("COMMIT")?;
                return Ok(None);
            };
            let id = parse_uuid(&id_text)?;
            let changed = conn.execute(
                "UPDATE jobs SET state = 'STARTING' WHERE id = ?1 AND state = 'QUEUED'",
                [id.to_string()],
            )?;
            if changed != 1 {
                return Err(StoreError::InvalidTransition {
                    id,
                    state: JobState::Queued,
                    action: "claim",
                });
            }
            // started_at is deliberately left NULL until the process is spawned;
            // claim only owns the execution slot and enters STARTING.
            let job = get_job_with(&conn, id)?;
            conn.execute_batch("COMMIT")?;
            Ok(Some(job))
        })();
        if result.is_err() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }

    pub fn set_running(
        &self,
        id: Uuid,
        pid: u32,
        execution_dir: impl AsRef<Path>,
    ) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        self.transition_job(&mut conn, id, "start", &[JobState::Starting], |conn| {
            conn.execute(
                "UPDATE jobs SET state = 'RUNNING', started_at = ?2, pid = ?3, execution_dir = ?4
                 WHERE id = ?1 AND state = 'STARTING'",
                params![
                    id.to_string(),
                    Utc::now().to_rfc3339(),
                    i64::from(pid),
                    execution_dir.as_ref().to_string_lossy().as_ref(),
                ],
            )
        })
    }

    pub fn finish(
        &self,
        id: Uuid,
        exit_code: Option<i32>,
        failure_detail: Option<&str>,
    ) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let current = self.current_state(&tx, id)?;
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
        tx.execute(
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
        let job = get_job_with(&tx, id)?;
        tx.commit()?;
        Ok(job)
    }

    /// Clear process-specific fields after execution cleanup while retaining
    /// the terminal result and durable failure detail.
    pub fn clear_runtime(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let state = self.current_state(&tx, id)?;
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
        tx.execute(
            "UPDATE jobs SET pid = NULL, execution_dir = NULL WHERE id = ?1",
            [id.to_string()],
        )?;
        let job = get_job_with(&tx, id)?;
        tx.commit()?;
        Ok(job)
    }

    /// Preserve a cleanup or execution diagnostic on a terminal job without
    /// changing its terminal state or exit result.
    pub fn record_failure_detail(&self, id: Uuid, detail: &str) -> Result<Job, StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE jobs SET failure_detail = ?2 WHERE id = ?1",
            params![id.to_string(), detail],
        )?;
        get_job_with(&conn, id)
    }

    pub fn mark_runtime_jobs_lost(&self) -> Result<(), StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE jobs SET state = 'LOST', finished_at = ?1
             WHERE state IN ('STARTING', 'RUNNING', 'CANCELLING')",
            [Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::Poisoned)
    }

    fn current_state(&self, conn: &Connection, id: Uuid) -> Result<JobState, StoreError> {
        let value: Option<String> = conn
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
        conn: &mut Connection,
        id: Uuid,
        action: &'static str,
        allowed: &[JobState],
        update: F,
    ) -> Result<Job, StoreError>
    where
        F: FnOnce(&Connection) -> Result<usize, rusqlite::Error>,
    {
        let tx = conn.transaction()?;
        let state = self.current_state(&tx, id)?;
        if !allowed.contains(&state) {
            return Err(StoreError::InvalidTransition { id, state, action });
        }
        if update(&tx)? != 1 {
            return Err(StoreError::InvalidTransition { id, state, action });
        }
        let job = get_job_with(&tx, id)?;
        tx.commit()?;
        Ok(job)
    }
}

fn get_job_with(conn: &Connection, id: Uuid) -> Result<Job, StoreError> {
    conn.query_row(
        "SELECT id,name,user,repository,git_commit,cwd,command,state,created_at,
                committed_at,started_at,finished_at,exit_code,pid,execution_dir,failure_detail
         FROM jobs WHERE id = ?1",
        [id.to_string()],
        row_to_job,
    )
    .optional()?
    .ok_or(StoreError::NotFound { id })
}

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    let id: String = row.get(0)?;
    let command: String = row.get(6)?;
    let state: String = row.get(7)?;
    let created_at: String = row.get(8)?;
    let committed_at: Option<String> = row.get(9)?;
    let started_at: Option<String> = row.get(10)?;
    let finished_at: Option<String> = row.get(11)?;
    let pid: Option<i64> = row.get(13)?;
    let parse = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .map(|time| time.with_timezone(&Utc))
            .map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })
    };
    let parse_opt = |value: Option<String>| value.as_deref().map(parse).transpose();
    Ok(Job {
        id: parse_uuid(&id).map_err(to_sql_error)?,
        name: row.get(1)?,
        user: row.get(2)?,
        repository: PathBuf::from(row.get::<_, String>(3)?),
        git_commit: row.get(4)?,
        cwd: PathBuf::from(row.get::<_, String>(5)?),
        command: serde_json::from_str(&command).map_err(to_sql_error)?,
        state: parse_state(&state).map_err(to_sql_error)?,
        created_at: parse(&created_at)?,
        committed_at: parse_opt(committed_at)?,
        started_at: parse_opt(started_at)?,
        finished_at: parse_opt(finished_at)?,
        exit_code: row.get(12)?,
        pid: pid
            .map(u32::try_from)
            .transpose()
            .map_err(|err| to_sql_error(err.to_string()))?,
        execution_dir: row.get::<_, Option<String>>(14)?.map(PathBuf::from),
        failure_detail: row.get(15)?,
    })
}

fn parse_uuid(value: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value).map_err(|err| StoreError::InvalidData(err.to_string()))
}

fn parse_state(value: &str) -> Result<JobState, StoreError> {
    JobState::from_str(value).map_err(StoreError::InvalidData)
}

fn to_sql_error<E: std::fmt::Display>(error: E) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}
