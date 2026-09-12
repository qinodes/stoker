use rusqlite::Transaction;

use super::super::error::StoreError;

const COLUMNS: &[(&str, &str)] = &[
    ("log_max_bytes_per_job", "INTEGER NOT NULL DEFAULT 67108864"),
    ("log_segment_bytes", "INTEGER NOT NULL DEFAULT 1048576"),
    ("log_max_bytes_total", "INTEGER NOT NULL DEFAULT 1073741824"),
    ("log_retention_jobs", "INTEGER NOT NULL DEFAULT 100"),
    (
        "log_disk_reserve_bytes",
        "INTEGER NOT NULL DEFAULT 536870912",
    ),
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
