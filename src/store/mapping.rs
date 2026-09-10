use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::domain::{Job, JobState};

use super::error::StoreError;
use super::schema::JOB_COLUMNS;

pub(super) fn storage_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn get_job_with(conn: &Connection, id: Uuid) -> Result<Job, StoreError> {
    conn.query_row(
        &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = ?1"),
        [id.to_string()],
        row_to_job,
    )
    .optional()?
    .ok_or(StoreError::NotFound { id })
}

pub(super) fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
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
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
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
            .map_err(|error| to_sql_error(error.to_string()))?,
        failure_detail: row.get(14)?,
        description: row.get(15)?,
        description_revision: row.get(16)?,
    })
}

pub(super) fn parse_uuid(value: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(value).map_err(|error| StoreError::InvalidData(error.to_string()))
}

pub(super) fn parse_state(value: &str) -> Result<JobState, StoreError> {
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
