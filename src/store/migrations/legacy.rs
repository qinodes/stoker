use std::path::PathBuf;

use rusqlite::{Transaction, params};

use crate::config::normalize_path;

use super::super::error::StoreError;
use super::super::mapping::storage_path;
use super::super::schema::BASE_SCHEMA;
use super::table_columns;

pub(super) fn bootstrap(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let jobs_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'jobs')",
        [],
        |row| row.get(0),
    )?;
    if !jobs_exists {
        transaction.execute_batch(BASE_SCHEMA)?;
        return Ok(());
    }

    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            queue_locked INTEGER NOT NULL DEFAULT 0 CHECK (queue_locked IN (0, 1)),
            log_max_bytes_per_job INTEGER NOT NULL DEFAULT 67108864,
            log_segment_bytes INTEGER NOT NULL DEFAULT 1048576,
            log_max_bytes_total INTEGER NOT NULL DEFAULT 1073741824,
            log_retention_jobs INTEGER NOT NULL DEFAULT 100,
            log_disk_reserve_bytes INTEGER NOT NULL DEFAULT 536870912,
            termination_grace_ms INTEGER NOT NULL DEFAULT 500,
            max_runtime_ms INTEGER,
            startup_timeout_ms INTEGER NOT NULL DEFAULT 30000
        );
        INSERT OR IGNORE INTO settings
            (id, queue_locked, log_max_bytes_per_job, log_segment_bytes,
             log_max_bytes_total, log_retention_jobs, log_disk_reserve_bytes)
        VALUES (1, 0, 67108864, 1048576, 1073741824, 100, 536870912);",
    )?;

    let columns = table_columns(transaction, "jobs")?;
    if !columns.iter().any(|name| name == "repository") {
        return Ok(());
    }
    let queue_order = if columns.iter().any(|name| name == "queue_order") {
        "queue_order"
    } else {
        "NULL"
    };
    let query = format!(
        "SELECT id,name,user,repository,cwd,command,state,{queue_order},created_at,
                committed_at,started_at,finished_at,exit_code,pid,failure_detail
         FROM jobs"
    );
    let legacy_rows = transaction
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

    transaction.execute_batch(
        "CREATE TABLE jobs_v1 (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            user TEXT NOT NULL,
            cwd TEXT NOT NULL,
            command TEXT NOT NULL,
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
        transaction.execute(
            "INSERT INTO jobs_v1
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
    transaction.execute_batch("DROP TABLE jobs; ALTER TABLE jobs_v1 RENAME TO jobs;")?;
    Ok(())
}
