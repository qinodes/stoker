//! Workspace execution mode and scheduled concurrency policy.

use rusqlite::{TransactionBehavior, params};

use crate::domain::flow::ExecutionMode;

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{
    has_active_work, parse_mode, require_queue_locked, require_scheduled_mode,
};

impl Store {
    pub fn set_mode(&self, mode: ExecutionMode) -> Result<ExecutionMode, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_queue_locked(&transaction)?;
        if has_active_work(&transaction)? {
            return Err(StoreError::InvalidData(
                "cannot change mode while an execution is active or cleanup is incomplete".into(),
            ));
        }
        transaction.execute(
            "UPDATE settings SET mode = ?1 WHERE id = 1",
            [mode.to_string()],
        )?;
        transaction.commit()?;
        Ok(mode)
    }

    pub fn set_scheduled_concurrency(&self, value: u32) -> Result<u32, StoreError> {
        self.set_scheduled_concurrency_inner(value, false)
    }

    /// Web scheduled-mode writes repeat the mode check inside the immediate
    /// transaction so a mode change cannot race a request that passed the
    /// adapter's initial guard.
    pub fn set_scheduled_concurrency_for_scheduled_workspace(
        &self,
        value: u32,
    ) -> Result<u32, StoreError> {
        self.set_scheduled_concurrency_inner(value, true)
    }

    fn set_scheduled_concurrency_inner(
        &self,
        value: u32,
        require_scheduled: bool,
    ) -> Result<u32, StoreError> {
        if value == 0 {
            return Err(StoreError::InvalidData(
                "max concurrency must be a positive integer".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if require_scheduled {
            require_scheduled_mode(&transaction)?;
        }
        require_queue_locked(&transaction)?;
        if has_active_work(&transaction)? {
            return Err(StoreError::InvalidData("cannot change max concurrency while an execution is active or cleanup is incomplete".into()));
        }
        transaction.execute(
            "UPDATE settings SET max_concurrency = ?1 WHERE id = 1",
            params![i64::from(value)],
        )?;
        transaction.commit()?;
        Ok(value)
    }

    pub fn current_mode(&self) -> Result<ExecutionMode, StoreError> {
        let connection = self.lock()?;
        let mode: String =
            connection.query_row("SELECT mode FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        parse_mode(&mode)
    }

    pub fn queue_recovery_fence(&self) -> Result<bool, StoreError> {
        let connection = self.lock()?;
        Ok(connection.query_row(
            "SELECT recovery_fence FROM settings WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    pub fn scheduled_concurrency(&self) -> Result<u32, StoreError> {
        let connection = self.lock()?;
        let value: i64 = connection.query_row(
            "SELECT max_concurrency FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        u32::try_from(value)
            .map_err(|_| StoreError::InvalidData(format!("invalid max concurrency {value}")))
    }
}
