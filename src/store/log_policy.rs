use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::config::LogPolicy;
use crate::domain::JobState;

use super::connection::Store;
use super::error::StoreError;
use super::mapping::parse_uuid;

impl Store {
    pub fn log_policy(&self) -> Result<LogPolicy, StoreError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT log_max_bytes_per_job, log_segment_bytes,
                        log_max_bytes_total, log_retention_jobs,
                        log_disk_reserve_bytes
                 FROM settings WHERE id = 1",
                [],
                |row| {
                    Ok(LogPolicy {
                        max_bytes_per_job: row
                            .get::<_, i64>(0)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        segment_bytes: row
                            .get::<_, i64>(1)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        max_bytes_total: row
                            .get::<_, i64>(2)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        retention_jobs: row
                            .get::<_, i64>(3)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        disk_reserve_bytes: row
                            .get::<_, i64>(4)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    })
                },
            )
            .map_err(StoreError::from)
    }

    /// Atomically update the policy only while queue claiming is fenced and no
    /// runtime execution can still be using the old limits.
    pub fn set_log_policy(&self, policy: LogPolicy) -> Result<(), StoreError> {
        policy.validate().map_err(StoreError::InvalidData)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let locked: i64 = transaction.query_row(
            "SELECT queue_locked FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        if locked == 0 {
            return Err(StoreError::QueueUnlocked);
        }
        let active: Option<(String, String)> = transaction
            .query_row(
                "SELECT id, state FROM jobs
                 WHERE state IN ('STARTING', 'RUNNING', 'CANCELLING')
                 ORDER BY started_at, id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id_text, state_text)) = active {
            let id = parse_uuid(&id_text)?;
            let state = state_text
                .parse::<JobState>()
                .map_err(StoreError::InvalidData)?;
            return Err(StoreError::ActiveJob { id, state });
        }
        transaction.execute(
            "UPDATE settings SET
                 log_max_bytes_per_job = ?1,
                 log_segment_bytes = ?2,
                 log_max_bytes_total = ?3,
                 log_retention_jobs = ?4,
                 log_disk_reserve_bytes = ?5
             WHERE id = 1",
            params![
                i64::try_from(policy.max_bytes_per_job).map_err(|_| StoreError::InvalidData(
                    "log max bytes per job is too large".into()
                ))?,
                i64::try_from(policy.segment_bytes).map_err(|_| StoreError::InvalidData(
                    "log segment bytes is too large".into()
                ))?,
                i64::try_from(policy.max_bytes_total).map_err(|_| StoreError::InvalidData(
                    "global log max bytes is too large".into()
                ))?,
                i64::try_from(policy.retention_jobs).map_err(|_| StoreError::InvalidData(
                    "log retention jobs is too large".into()
                ))?,
                i64::try_from(policy.disk_reserve_bytes).map_err(|_| StoreError::InvalidData(
                    "log disk reserve bytes is too large".into()
                ))?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
