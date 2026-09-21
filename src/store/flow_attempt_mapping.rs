//! Attempt-row mapping kept separate from schedule and runtime helpers.

use rusqlite::Row;
use uuid::Uuid;

use crate::domain::flow::{Attempt, AttemptState};

use super::flow_mapping::parse_datetime;

pub(super) fn attempt_from_row(row: &Row<'_>) -> rusqlite::Result<Attempt> {
    let attempt_id: String = row.get(0)?;
    let run_id: String = row.get(1)?;
    let number: i64 = row.get(3)?;
    let state: String = row.get(4)?;
    let started_at: Option<String> = row.get(8)?;
    let finished_at: Option<String> = row.get(9)?;
    Ok(Attempt {
        attempt_id: Uuid::parse_str(&attempt_id).map_err(|_| rusqlite::Error::InvalidQuery)?,
        run_id: Uuid::parse_str(&run_id).map_err(|_| rusqlite::Error::InvalidQuery)?,
        task_id: row.get(2)?,
        number: u32::try_from(number).map_err(|_| rusqlite::Error::InvalidQuery)?,
        state: serde_json::from_str::<AttemptState>(&format!("\"{state}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        exit_code: row.get(5)?,
        failure_kind: row.get(6)?,
        failure_detail: row.get(7)?,
        started_at: started_at
            .map(|value| parse_datetime(&value).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?,
        finished_at: finished_at
            .map(|value| parse_datetime(&value).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?,
    })
}
