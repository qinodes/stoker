use rusqlite::Transaction;

use super::super::error::StoreError;
use super::table_columns;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let columns = table_columns(transaction, "jobs")?;
    if !columns.iter().any(|name| name == "description") {
        transaction.execute("ALTER TABLE jobs ADD COLUMN description TEXT", [])?;
    }
    if !columns.iter().any(|name| name == "description_revision") {
        transaction.execute(
            "ALTER TABLE jobs ADD COLUMN description_revision INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}
