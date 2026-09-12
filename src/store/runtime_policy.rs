use rusqlite::OptionalExtension;

use crate::config::RuntimePolicy;

use super::{Store, StoreError};

impl Store {
    pub fn runtime_policy(&self) -> Result<RuntimePolicy, StoreError> {
        let connection = self.lock()?;
        let values = connection.query_row(
            "SELECT termination_grace_ms, max_runtime_ms, startup_timeout_ms
             FROM settings WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        let policy = RuntimePolicy {
            termination_grace_ms: u64::try_from(values.0)
                .map_err(|_| StoreError::InvalidData("invalid termination grace".into()))?,
            max_runtime_ms: values
                .1
                .map(u64::try_from)
                .transpose()
                .map_err(|_| StoreError::InvalidData("invalid maximum runtime".into()))?,
            startup_timeout_ms: u64::try_from(values.2)
                .map_err(|_| StoreError::InvalidData("invalid startup timeout".into()))?,
        };
        policy.validate().map_err(StoreError::InvalidData)?;
        Ok(policy)
    }

    pub fn set_runtime_policy(&self, policy: RuntimePolicy) -> Result<(), StoreError> {
        policy.validate().map_err(StoreError::InvalidData)?;
        let mut connection = self.lock()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
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
                 ORDER BY started_at, created_at, id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, state)) = active {
            let id = uuid::Uuid::parse_str(&id)
                .map_err(|_| StoreError::InvalidData("invalid active job id".into()))?;
            let state = crate::domain::parse_job_state(&state)
                .map_err(|error| StoreError::InvalidData(error.to_string()))?;
            return Err(StoreError::ActiveJob { id, state });
        }
        transaction.execute(
            "UPDATE settings SET termination_grace_ms = ?1,
             max_runtime_ms = ?2, startup_timeout_ms = ?3 WHERE id = 1",
            rusqlite::params![
                i64::try_from(policy.termination_grace_ms).map_err(|_| StoreError::InvalidData(
                    "termination grace is too large".into()
                ))?,
                policy
                    .max_runtime_ms
                    .map(i64::try_from)
                    .transpose()
                    .map_err(|_| StoreError::InvalidData("maximum runtime is too large".into()))?,
                i64::try_from(policy.startup_timeout_ms)
                    .map_err(|_| StoreError::InvalidData("startup timeout is too large".into()))?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
