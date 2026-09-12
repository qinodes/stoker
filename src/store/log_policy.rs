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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::POLICY_MB_BYTES;
    use tempfile::TempDir;

    fn test_store() -> Store {
        let directory = Box::leak(Box::new(TempDir::new().unwrap()));
        Store::open(directory.path().join("stoker.db")).unwrap()
    }

    #[test]
    fn invalid_log_policies_are_rejected_before_opening_a_transaction() {
        let store = test_store();
        let defaults = LogPolicy::default();
        for (policy, expected) in [
            (
                LogPolicy {
                    max_bytes_per_job: 0,
                    ..defaults
                },
                "log max bytes per job must be greater than zero",
            ),
            (
                LogPolicy {
                    segment_bytes: 0,
                    ..defaults
                },
                "log segment bytes must be greater than zero",
            ),
            (
                LogPolicy {
                    segment_bytes: defaults.max_bytes_per_job / 2 + 1,
                    ..defaults
                },
                "log segment bytes cannot exceed half of the shared per-job log limit",
            ),
            (
                LogPolicy {
                    max_bytes_total: defaults.max_bytes_per_job - 1,
                    ..defaults
                },
                "global log limit cannot be smaller than the per-job log limit",
            ),
            (
                LogPolicy {
                    disk_reserve_bytes: 0,
                    ..defaults
                },
                "log disk reserve bytes must be greater than zero",
            ),
        ] {
            assert!(matches!(
                store.set_log_policy(policy),
                Err(StoreError::InvalidData(message)) if message == expected
            ));
        }
    }

    #[test]
    fn oversized_log_policy_values_report_the_field_that_cannot_be_stored() {
        let store = test_store();
        store.lock_queue().unwrap();
        let defaults = LogPolicy::default();
        let cases = [
            (
                LogPolicy {
                    max_bytes_per_job: u64::MAX,
                    max_bytes_total: u64::MAX,
                    ..defaults
                },
                "log max bytes per job is too large",
            ),
            (
                LogPolicy {
                    max_bytes_total: u64::MAX,
                    ..defaults
                },
                "global log max bytes is too large",
            ),
            (
                LogPolicy {
                    retention_jobs: u64::MAX,
                    ..defaults
                },
                "log retention jobs is too large",
            ),
            (
                LogPolicy {
                    disk_reserve_bytes: u64::MAX,
                    ..defaults
                },
                "log disk reserve bytes is too large",
            ),
        ];
        for (policy, expected) in cases {
            assert!(matches!(
                store.set_log_policy(policy),
                Err(StoreError::InvalidData(message)) if message == expected
            ));
        }
    }

    #[test]
    fn negative_database_values_are_rejected_when_reading_log_policy() {
        let store = test_store();
        for column in [
            "log_max_bytes_per_job",
            "log_segment_bytes",
            "log_max_bytes_total",
            "log_retention_jobs",
            "log_disk_reserve_bytes",
        ] {
            let connection = store.lock().unwrap();
            connection
                .execute(
                    &format!("UPDATE settings SET {column} = -1 WHERE id = 1"),
                    [],
                )
                .unwrap();
            drop(connection);
            assert!(matches!(store.log_policy(), Err(StoreError::Database(_))));
            let connection = store.lock().unwrap();
            connection
                .execute(
                    &format!("UPDATE settings SET {column} = ?1 WHERE id = 1"),
                    [i64::try_from(POLICY_MB_BYTES).unwrap()],
                )
                .unwrap();
        }
    }
}
