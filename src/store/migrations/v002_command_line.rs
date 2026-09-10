use rusqlite::Transaction;

use super::super::error::StoreError;
use super::table_columns;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let columns = table_columns(transaction, "jobs")?;
    if !columns.iter().any(|name| name == "command_line") {
        transaction.execute("ALTER TABLE jobs ADD COLUMN command_line TEXT", [])?;
    }
    Ok(())
}
