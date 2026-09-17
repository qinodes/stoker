use rusqlite::Transaction;

use super::super::error::StoreError;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    for table in ["jobs", "flow_definitions"] {
        for (name, definition) in [
            ("period_value", "INTEGER"),
            ("period_unit", "TEXT"),
            ("period_first_at_utc", "TEXT"),
        ] {
            if !super::table_columns(transaction, table)?
                .iter()
                .any(|column| column == name)
            {
                transaction.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
    }

    let occurrences_sql: String = transaction.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'occurrences'",
        [],
        |row| row.get(0),
    )?;
    if !occurrences_sql.contains("'periodic'") {
        transaction.execute_batch(
            r#"
DROP INDEX IF EXISTS occurrences_due;
ALTER TABLE occurrences RENAME TO occurrences_v9;

CREATE TABLE occurrences (
    occurrence_id TEXT PRIMARY KEY NOT NULL,
    internal_definition_id TEXT NOT NULL,
    flow_id TEXT NOT NULL,
    schedule_generation INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('once', 'daily', 'periodic')),
    occurrence_key TEXT NOT NULL,
    local_date TEXT,
    due_at TEXT NOT NULL,
    state TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (internal_definition_id, schedule_generation, kind, occurrence_key)
);

INSERT INTO occurrences (
    occurrence_id, internal_definition_id, flow_id, schedule_generation,
    kind, occurrence_key, local_date, due_at, state, reason, created_at
)
SELECT
    occurrence_id, internal_definition_id, flow_id, schedule_generation,
    kind, occurrence_key, local_date, due_at, state, reason, created_at
FROM occurrences_v9;

DROP TABLE occurrences_v9;
CREATE INDEX occurrences_due ON occurrences(state, due_at, flow_id);
            "#,
        )?;
    }
    Ok(())
}
