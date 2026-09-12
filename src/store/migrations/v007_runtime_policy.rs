use rusqlite::Transaction;

use super::super::error::StoreError;

const COLUMNS: &[(&str, &str)] = &[
    ("termination_grace_ms", "INTEGER NOT NULL DEFAULT 500"),
    ("max_runtime_ms", "INTEGER"),
    ("startup_timeout_ms", "INTEGER NOT NULL DEFAULT 30000"),
];

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let columns = super::table_columns(transaction, "settings")?;
    for (name, definition) in COLUMNS {
        if !columns.iter().any(|column| column == name) {
            transaction.execute(
                &format!("ALTER TABLE settings ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    Ok(())
}
