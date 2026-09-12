use std::path::Path;

use super::{Store, StoreError};

impl Store {
    pub fn quick_check(&self) -> Result<(), StoreError> {
        self.check_pragma("quick_check")
    }

    pub fn integrity_check(&self) -> Result<(), StoreError> {
        self.check_pragma("integrity_check")
    }

    /// Create a consistent SQLite backup, including WAL contents, without
    /// requiring callers to copy the live database file themselves.
    pub fn backup_to(&self, destination: &Path) -> Result<(), StoreError> {
        let connection = self.lock()?;
        connection.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
        connection.execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        Ok(())
    }

    fn check_pragma(&self, pragma: &str) -> Result<(), StoreError> {
        let connection = self.lock()?;
        let result: String =
            connection.query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))?;
        if result.eq_ignore_ascii_case("ok") {
            Ok(())
        } else {
            Err(StoreError::InvalidData(format!(
                "SQLite {pragma} reported {result}"
            )))
        }
    }
}
