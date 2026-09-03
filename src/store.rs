use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use thiserror::Error;
use uuid::Uuid;

use crate::config::normalize_path;
use crate::domain::{Job, JobState, NewJob};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    user TEXT NOT NULL,
    cwd TEXT NOT NULL,
    command TEXT NOT NULL,
    command_line TEXT,
    state TEXT NOT NULL,
    queue_order INTEGER,
    created_at TEXT NOT NULL,
    committed_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    exit_code INTEGER,
    pid INTEGER,
    failure_detail TEXT
);
"#;

const INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS jobs_state_queue_order_id
    ON jobs (state, queue_order, id);
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
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // Allow service writes while CLI commands poll job state.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(SCHEMA)?;
        migrate_legacy_git_schema(&mut connection)?;
        migrate_command_line(&connection)?;
        migrate_queue_order(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

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
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        let command = serde_json::to_string(&new_job.command)?;
        let cwd = storage_path(&new_job.cwd);
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO jobs (id,name,user,cwd,command,command_line,state,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id.to_string(),
                new_job.name,
                new_job.user,
                cwd,
                command,
                command_line,
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
        self.list_jobs_with_state(owner, None)
    }

    pub fn list_jobs_with_state(
        &self,
        owner: Option<&str>,
        state: Option<JobState>,
    ) -> Result<Vec<Job>, StoreError> {
        let conn = self.lock()?;
        let mut statement = match (owner.is_some(), state) {
            (true, Some(_)) => conn.prepare(
                "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,failure_detail
                 FROM jobs WHERE user = ?1 AND state = ?2
                 ORDER BY CASE WHEN state = 'QUEUED' THEN 0 ELSE 1 END,
                          CASE WHEN state = 'QUEUED' THEN queue_order END,
                          CASE WHEN state <> 'QUEUED' THEN COALESCE(committed_at, created_at) END DESC,
                          id",
            )?,
            (false, Some(_)) => conn.prepare(
                "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,failure_detail
                 FROM jobs WHERE state = ?1
                 ORDER BY CASE WHEN state = 'QUEUED' THEN 0 ELSE 1 END,
                          CASE WHEN state = 'QUEUED' THEN queue_order END,
                          CASE WHEN state <> 'QUEUED' THEN COALESCE(committed_at, created_at) END DESC,
                          id",
            )?,
            (true, None) => conn.prepare(
                "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,failure_detail
                 FROM jobs WHERE user = ?1
                 ORDER BY CASE WHEN state = 'QUEUED' THEN 0 ELSE 1 END,
                          CASE WHEN state = 'QUEUED' THEN queue_order END,
                          CASE WHEN state <> 'QUEUED' THEN COALESCE(committed_at, created_at) END DESC,
                          id",
            )?,
            (false, None) => conn.prepare(
                "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,failure_detail
                 FROM jobs
                 ORDER BY CASE WHEN state = 'QUEUED' THEN 0 ELSE 1 END,
                          CASE WHEN state = 'QUEUED' THEN queue_order END,
                          CASE WHEN state <> 'QUEUED' THEN COALESCE(committed_at, created_at) END DESC,
                          id",
            )?,
        };
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

    /// Delete all terminal jobs and return their records so callers can remove
    /// associated filesystem artifacts.
    pub fn clean_terminal_jobs(&self) -> Result<Vec<Job>, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let jobs = {
            let mut statement = tx.prepare(
                "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                        committed_at,started_at,finished_at,exit_code,pid,failure_detail
                 FROM jobs
                 WHERE state IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'LOST')",
            )?;
            statement
                .query_map([], row_to_job)?
                .collect::<Result<Vec<_>, _>>()?
        };
        for job in &jobs {
            tx.execute("DELETE FROM jobs WHERE id = ?1", [job.id.to_string()])?;
        }
        tx.commit()?;
        Ok(jobs)
    }

    pub fn commit_job(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = self.current_state(&tx, id)?;
        if state != JobState::Draft {
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "commit",
            });
        }
        let queue_order = next_queue_order(&tx)?;
        if tx.execute(
            "UPDATE jobs SET state = 'QUEUED', queue_order = ?2, committed_at = ?3
             WHERE id = ?1 AND state = 'DRAFT'",
            params![id.to_string(), queue_order, Utc::now().to_rfc3339()],
        )? != 1
        {
            return Err(StoreError::InvalidTransition {
                id,
                state: JobState::Draft,
                action: "commit",
            });
        }
        normalize_queue(&tx)?;
        let job = get_job_with(&tx, id)?;
        tx.commit()?;
        Ok(job)
    }

    pub fn cancel_not_started(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = self.current_state(&tx, id)?;
        if !matches!(state, JobState::Draft | JobState::Queued) {
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "cancel",
            });
        }
        if tx.execute(
            "UPDATE jobs SET state = 'CANCELLED', queue_order = NULL, finished_at = ?2
             WHERE id = ?1 AND state IN ('DRAFT', 'QUEUED')",
            params![id.to_string(), Utc::now().to_rfc3339()],
        )? != 1
        {
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "cancel",
            });
        }
        if state == JobState::Queued {
            normalize_queue(&tx)?;
        }
        let job = get_job_with(&tx, id)?;
        tx.commit()?;
        Ok(job)
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
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let selected: Option<String> = tx
            .query_row(
                "SELECT id FROM jobs WHERE state = 'QUEUED'
                 ORDER BY queue_order, id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id_text) = selected else {
            tx.commit()?;
            return Ok(None);
        };
        let id = parse_uuid(&id_text)?;
        if tx.execute(
            "UPDATE jobs SET state = 'STARTING', queue_order = NULL
             WHERE id = ?1 AND state = 'QUEUED'",
            [id.to_string()],
        )? != 1
        {
            return Err(StoreError::InvalidTransition {
                id,
                state: JobState::Queued,
                action: "claim",
            });
        }
        normalize_queue(&tx)?;
        // started_at is deliberately left NULL until the process is spawned;
        // claim only owns the execution slot and enters STARTING.
        let job = get_job_with(&tx, id)?;
        tx.commit()?;
        Ok(Some(job))
    }

    pub fn set_running(&self, id: Uuid, pid: u32) -> Result<Job, StoreError> {
        let mut conn = self.lock()?;
        self.transition_job(&mut conn, id, "start", &[JobState::Starting], |conn| {
            conn.execute(
                "UPDATE jobs SET state = 'RUNNING', started_at = ?2, pid = ?3
                 WHERE id = ?1 AND state = 'STARTING'",
                params![id.to_string(), Utc::now().to_rfc3339(), i64::from(pid),],
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
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        tx.execute("UPDATE jobs SET pid = NULL WHERE id = ?1", [id.to_string()])?;
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
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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

fn migrate_queue_order(connection: &mut Connection) -> Result<(), StoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let has_queue_order = tx
        .prepare("PRAGMA table_info(jobs)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "queue_order");
    if !has_queue_order {
        tx.execute("ALTER TABLE jobs ADD COLUMN queue_order INTEGER", [])?;
    }
    tx.execute_batch(INDEXES)?;
    let needs_backfill: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM jobs WHERE state = 'QUEUED' AND queue_order IS NULL
         )",
        [],
        |row| row.get(0),
    )?;
    if needs_backfill {
        let ids = tx
            .prepare(
                "SELECT id FROM jobs WHERE state = 'QUEUED'
                 ORDER BY committed_at, id",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for (index, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE jobs SET queue_order = ?2 WHERE id = ?1",
                params![id, i64::try_from(index + 1).expect("queue length fits i64")],
            )?;
        }
    }
    tx.execute(
        "UPDATE jobs SET queue_order = NULL WHERE state <> 'QUEUED'",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_legacy_git_schema(connection: &mut Connection) -> Result<(), StoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let columns = tx
        .prepare("PRAGMA table_info(jobs)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|name| name == "repository") {
        tx.commit()?;
        return Ok(());
    }
    let has_queue_order = columns.iter().any(|name| name == "queue_order");
    let queue_order = if has_queue_order {
        "queue_order"
    } else {
        "NULL"
    };
    let query = format!(
        "SELECT id,name,user,repository,cwd,command,state,{queue_order},created_at,
                committed_at,started_at,finished_at,exit_code,pid,failure_detail
         FROM jobs"
    );
    let legacy_rows = tx
        .prepare(&query)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<i32>>(12)?,
                row.get::<_, Option<i64>>(13)?,
                row.get::<_, Option<String>>(14)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    tx.execute_batch(
        "CREATE TABLE jobs_v2 (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            user TEXT NOT NULL,
            cwd TEXT NOT NULL,
            command TEXT NOT NULL,
            command_line TEXT,
            state TEXT NOT NULL,
            queue_order INTEGER,
            created_at TEXT NOT NULL,
            committed_at TEXT,
            started_at TEXT,
            finished_at TEXT,
            exit_code INTEGER,
            pid INTEGER,
            failure_detail TEXT
        );
        DROP INDEX IF EXISTS jobs_state_commit_id;
        DROP INDEX IF EXISTS jobs_state_queue_order_id;",
    )?;
    for (
        id,
        name,
        user,
        repository,
        legacy_cwd,
        command,
        state,
        queue_order,
        created_at,
        committed_at,
        started_at,
        finished_at,
        exit_code,
        pid,
        failure_detail,
    ) in legacy_rows
    {
        let raw_cwd = PathBuf::from(legacy_cwd);
        let cwd = if raw_cwd.is_absolute() {
            normalize_path(raw_cwd)
        } else {
            normalize_path(PathBuf::from(repository).join(raw_cwd))
        };
        tx.execute(
            "INSERT INTO jobs_v2
                (id,name,user,cwd,command,state,queue_order,created_at,committed_at,
                 started_at,finished_at,exit_code,pid,failure_detail)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                id,
                name,
                user,
                storage_path(&cwd),
                command,
                state,
                queue_order,
                created_at,
                committed_at,
                started_at,
                finished_at,
                exit_code,
                pid,
                failure_detail,
            ],
        )?;
    }
    tx.execute_batch("DROP TABLE jobs; ALTER TABLE jobs_v2 RENAME TO jobs;")?;
    tx.commit()?;
    Ok(())
}

fn migrate_command_line(connection: &Connection) -> Result<(), StoreError> {
    let has_command_line = connection
        .prepare("PRAGMA table_info(jobs)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "command_line");
    if !has_command_line {
        connection.execute("ALTER TABLE jobs ADD COLUMN command_line TEXT", [])?;
    }
    Ok(())
}

fn next_queue_order(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM jobs WHERE state = 'QUEUED'",
        [],
        |row| row.get(0),
    )?)
}

fn normalize_queue(conn: &Connection) -> Result<(), StoreError> {
    let ids = conn
        .prepare(
            "SELECT id FROM jobs WHERE state = 'QUEUED'
             ORDER BY queue_order, id",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE jobs SET queue_order = ?2 WHERE id = ?1",
            params![id, i64::try_from(index + 1).expect("queue length fits i64")],
        )?;
    }
    Ok(())
}

fn storage_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn get_job_with(conn: &Connection, id: Uuid) -> Result<Job, StoreError> {
    conn.query_row(
        "SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
                committed_at,started_at,finished_at,exit_code,pid,failure_detail
         FROM jobs WHERE id = ?1",
        [id.to_string()],
        row_to_job,
    )
    .optional()?
    .ok_or(StoreError::NotFound { id })
}

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    let id: String = row.get(0)?;
    let command: String = row.get(4)?;
    let command_line: Option<String> = row.get(5)?;
    let state: String = row.get(6)?;
    let queue_order: Option<i64> = row.get(7)?;
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
        cwd: PathBuf::from(row.get::<_, String>(3)?),
        command: serde_json::from_str(&command).map_err(to_sql_error)?,
        command_line,
        state: parse_state(&state).map_err(to_sql_error)?,
        queue_order,
        created_at: parse(&created_at)?,
        committed_at: parse_opt(committed_at)?,
        started_at: parse_opt(started_at)?,
        finished_at: parse_opt(finished_at)?,
        exit_code: row.get(12)?,
        pid: pid
            .map(u32::try_from)
            .transpose()
            .map_err(|err| to_sql_error(err.to_string()))?,
        failure_detail: row.get(14)?,
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
