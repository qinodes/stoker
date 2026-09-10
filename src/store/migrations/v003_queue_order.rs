use rusqlite::{Transaction, params};

use super::super::error::StoreError;
use super::super::schema::INDEXES;
use super::table_columns;

pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let columns = table_columns(transaction, "jobs")?;
    if !columns.iter().any(|name| name == "queue_order") {
        transaction.execute("ALTER TABLE jobs ADD COLUMN queue_order INTEGER", [])?;
    }
    transaction.execute_batch(INDEXES)?;
    let needs_backfill: bool = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM jobs WHERE state = 'QUEUED' AND queue_order IS NULL
         )",
        [],
        |row| row.get(0),
    )?;
    if needs_backfill {
        let ids = transaction
            .prepare(
                "SELECT id FROM jobs WHERE state = 'QUEUED'
                 ORDER BY committed_at, id",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for (index, id) in ids.iter().enumerate() {
            transaction.execute(
                "UPDATE jobs SET queue_order = ?2 WHERE id = ?1",
                params![id, i64::try_from(index + 1).expect("queue length fits i64")],
            )?;
        }
    }
    transaction.execute(
        "UPDATE jobs SET queue_order = NULL WHERE state <> 'QUEUED'",
        [],
    )?;
    Ok(())
}
