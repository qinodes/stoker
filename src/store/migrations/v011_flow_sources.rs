use rusqlite::Transaction;

use super::super::error::StoreError;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let columns = super::table_columns(transaction, "settings")?;
    for (name, definition) in [
        (
            "flow_source_mode",
            "TEXT NOT NULL DEFAULT 'manual' CHECK (flow_source_mode IN ('manual','sync'))",
        ),
        (
            "flow_definition_revision",
            "INTEGER NOT NULL DEFAULT 0 CHECK (flow_definition_revision >= 0)",
        ),
        ("flow_definition_hash", "TEXT NOT NULL DEFAULT ''"),
        ("flow_last_sync_at", "TEXT"),
        ("flow_last_source_hash", "TEXT"),
    ] {
        if !columns.iter().any(|column| column == name) {
            transaction.execute(
                &format!("ALTER TABLE settings ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    transaction.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS flow_sync_audit (
    revision INTEGER PRIMARY KEY NOT NULL,
    previous_hash TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    synced_at TEXT NOT NULL,
    added INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    removed INTEGER NOT NULL,
    unchanged INTEGER NOT NULL
);
"#,
    )?;
    Ok(())
}
