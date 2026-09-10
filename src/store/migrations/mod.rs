mod legacy;
mod v002_command_line;
mod v003_queue_order;
mod v004_description;
mod v005_text_constraints;

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::error::StoreError;

pub const CURRENT_SCHEMA_VERSION: u32 = 5;

type Migration = fn(&Transaction<'_>) -> Result<(), StoreError>;

const MIGRATIONS: &[(u32, Migration)] = &[
    (1, legacy::bootstrap),
    (2, v002_command_line::apply),
    (3, v003_queue_order::apply),
    (4, v004_description::apply),
    (5, v005_text_constraints::apply),
];

pub(super) fn schema_version(connection: &Connection) -> Result<u32, StoreError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    u32::try_from(version)
        .map_err(|_| StoreError::InvalidData(format!("invalid schema version {version}")))
}

pub(super) fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let mut version = schema_version(connection)?;
    reject_future_version(connection)?;
    for &(target, migration) in MIGRATIONS {
        if version >= target {
            continue;
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        migration(&transaction)?;
        transaction.pragma_update(None, "user_version", target)?;
        transaction.commit()?;
        version = target;
    }

    validate_latest_schema(connection)?;
    Ok(())
}

pub(super) fn reject_future_version(connection: &Connection) -> Result<(), StoreError> {
    let version = schema_version(connection)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(StoreError::InvalidData(format!(
            "database schema version {version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn validate_latest_schema(connection: &Connection) -> Result<(), StoreError> {
    let columns = table_columns(connection, "jobs")?;
    for required in [
        "id",
        "name",
        "user",
        "cwd",
        "command",
        "command_line",
        "state",
        "queue_order",
        "created_at",
        "description",
        "description_revision",
    ] {
        if !columns.iter().any(|column| column == required) {
            return Err(StoreError::InvalidData(format!(
                "schema version {CURRENT_SCHEMA_VERSION} is missing jobs.{required}"
            )));
        }
    }
    let settings_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
        [],
        |row| row.get(0),
    )?;
    if !settings_exists {
        return Err(StoreError::InvalidData(format!(
            "schema version {CURRENT_SCHEMA_VERSION} is missing settings"
        )));
    }
    let settings_row_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE id = 1)",
        [],
        |row| row.get(0),
    )?;
    if !settings_row_exists {
        return Err(StoreError::InvalidData(format!(
            "schema version {CURRENT_SCHEMA_VERSION} is missing settings row 1"
        )));
    }
    for index in ["jobs_state_queue_order_id", "jobs_user_state_created_at_id"] {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
            [index],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::InvalidData(format!(
                "schema version {CURRENT_SCHEMA_VERSION} is missing index {index}"
            )));
        }
    }
    let jobs_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'jobs'",
        [],
        |row| row.get(0),
    )?;
    for constraint in [
        "length(name) <= 128",
        "length(user) <= 50",
        "length(description) <= 200",
    ] {
        if !jobs_sql.contains(constraint) {
            return Err(StoreError::InvalidData(format!(
                "schema version {CURRENT_SCHEMA_VERSION} is missing jobs constraint {constraint}"
            )));
        }
    }
    Ok(())
}

pub(super) fn table_columns(
    connection: &Connection,
    table: &str,
) -> Result<Vec<String>, StoreError> {
    let query = format!("PRAGMA table_info({table})");
    Ok(connection
        .prepare(&query)?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?)
}
