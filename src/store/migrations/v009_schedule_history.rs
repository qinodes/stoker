use rusqlite::Transaction;

use super::super::error::StoreError;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    if !super::table_columns(transaction, "flow_definitions")?
        .iter()
        .any(|column| column == "daily_cursor_date")
    {
        transaction.execute(
            "ALTER TABLE flow_definitions ADD COLUMN daily_cursor_date TEXT",
            [],
        )?;
    }
    transaction.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS schedule_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    flow_id TEXT NOT NULL,
    internal_definition_id TEXT NOT NULL,
    schedule_generation INTEGER NOT NULL,
    event TEXT NOT NULL,
    detail TEXT,
    occurred_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS schedule_events_flow_time
    ON schedule_events(flow_id, occurred_at, event_id);
        "#,
    )?;
    Ok(())
}
