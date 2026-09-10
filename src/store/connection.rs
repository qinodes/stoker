use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;

use super::error::StoreError;
use super::migrations;

/// A reusable handle to one SQLite connection.
///
/// Clones share the same serialized connection, allowing composition roots to
/// construct the store once and pass inexpensive handles to adapters.
#[derive(Clone)]
pub struct Store {
    pub(super) connection: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        migrations::reject_future_version(&connection)?;
        // Allow service writes while CLI/UI commands poll job state.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrations::migrate(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn schema_version(&self) -> Result<u32, StoreError> {
        let connection = self.lock()?;
        migrations::schema_version(&connection)
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::Poisoned)
    }
}
