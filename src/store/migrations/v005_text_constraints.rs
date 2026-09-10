use rusqlite::{Transaction, params};

use crate::domain::{MAX_JOB_DESCRIPTION_LENGTH, MAX_JOB_NAME_LENGTH, MAX_JOB_USER_LENGTH};

use super::super::error::StoreError;
use super::super::schema::INDEXES;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let table_sql: Option<String> = transaction.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'jobs'",
        [],
        |row| row.get(0),
    )?;
    if table_sql.as_deref().is_some_and(has_current_constraints) {
        transaction.execute_batch(INDEXES)?;
        return Ok(());
    }

    let has_invalid_text: bool = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM jobs
             WHERE length(name) > ?1 OR length(user) > ?2 OR length(description) > ?3
         )",
        params![
            i64::try_from(MAX_JOB_NAME_LENGTH).expect("job name limit fits i64"),
            i64::try_from(MAX_JOB_USER_LENGTH).expect("job user limit fits i64"),
            i64::try_from(MAX_JOB_DESCRIPTION_LENGTH).expect("description limit fits i64")
        ],
        |row| row.get(0),
    )?;
    if has_invalid_text {
        return Err(StoreError::InvalidData(format!(
            "jobs table contains a name longer than {MAX_JOB_NAME_LENGTH} characters, a user longer than {MAX_JOB_USER_LENGTH} characters, or a description longer than {MAX_JOB_DESCRIPTION_LENGTH} characters"
        )));
    }

    transaction.execute_batch(
        "CREATE TABLE jobs_v5 (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL CHECK (length(name) <= 128),
            user TEXT NOT NULL CHECK (length(user) <= 50),
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
            failure_detail TEXT,
            description TEXT CHECK (description IS NULL OR length(description) <= 200),
            description_revision INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO jobs_v5
            (id,name,user,cwd,command,command_line,state,queue_order,created_at,
             committed_at,started_at,finished_at,exit_code,pid,failure_detail,
             description,description_revision)
        SELECT id,name,user,cwd,command,command_line,state,queue_order,created_at,
               committed_at,started_at,finished_at,exit_code,pid,failure_detail,
               description,description_revision
        FROM jobs;
        DROP INDEX IF EXISTS jobs_state_queue_order_id;
        DROP INDEX IF EXISTS jobs_user_state_created_at_id;
        DROP TABLE jobs;
        ALTER TABLE jobs_v5 RENAME TO jobs;",
    )?;
    transaction.execute_batch(INDEXES)?;
    Ok(())
}

fn has_current_constraints(sql: &str) -> bool {
    sql.contains("CHECK (length(name) <= 128)")
        && sql.contains("CHECK (length(user) <= 50)")
        && sql.contains("CHECK (description IS NULL OR length(description) <= 200)")
}
