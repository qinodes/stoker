mod legacy;
mod v002_command_line;
mod v003_queue_order;
mod v004_description;
mod v005_text_constraints;
mod v006_log_policy;
mod v007_runtime_policy;
mod v008_flows;
mod v009_schedule_history;
mod v010_periodic_schedules;

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::error::StoreError;

pub const CURRENT_SCHEMA_VERSION: u32 = 10;

type Migration = fn(&Transaction<'_>) -> Result<(), StoreError>;

const MIGRATIONS: &[(u32, Migration)] = &[
    (1, legacy::bootstrap),
    (2, v002_command_line::apply),
    (3, v003_queue_order::apply),
    (4, v004_description::apply),
    (5, v005_text_constraints::apply),
    (6, v006_log_policy::apply),
    (7, v007_runtime_policy::apply),
    (8, v008_flows::apply),
    (9, v009_schedule_history::apply),
    (10, v010_periodic_schedules::apply),
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
    for required in [
        "mode",
        "schedule_generation",
        "enabled",
        "retry",
        "internal_definition_id",
    ] {
        if !columns.iter().any(|column| column == required) {
            return Err(StoreError::InvalidData(format!(
                "schema version {CURRENT_SCHEMA_VERSION} is missing jobs.{required}"
            )));
        }
    }
    let settings_columns = table_columns(connection, "settings")?;
    for required in [
        "log_max_bytes_per_job",
        "log_segment_bytes",
        "log_max_bytes_total",
        "log_retention_jobs",
        "log_disk_reserve_bytes",
        "termination_grace_ms",
        "max_runtime_ms",
        "startup_timeout_ms",
        "mode",
        "max_concurrency",
        "dispatch_sequence",
        "recovery_fence",
    ] {
        if !settings_columns.iter().any(|column| column == required) {
            return Err(StoreError::InvalidData(format!(
                "schema version {CURRENT_SCHEMA_VERSION} is missing settings.{required}"
            )));
        }
    }
    for (table, required) in [
        ("flow_definitions", "flow_id"),
        ("flow_tasks", "task_id"),
        ("flow_dependencies", "upstream_task_id"),
        ("occurrences", "occurrence_id"),
        ("flow_runs", "run_id"),
        ("task_runs", "task_id"),
        ("attempts", "attempt_id"),
        ("manual_requests", "request_id"),
    ] {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists
            || !table_columns(connection, table)?
                .iter()
                .any(|column| column == required)
        {
            return Err(StoreError::InvalidData(format!(
                "schema version {CURRENT_SCHEMA_VERSION} is missing {table}.{required}"
            )));
        }
    }
    if !table_columns(connection, "flow_definitions")?
        .iter()
        .any(|column| column == "daily_cursor_date")
    {
        return Err(StoreError::InvalidData(format!(
            "schema version {CURRENT_SCHEMA_VERSION} is missing flow_definitions.daily_cursor_date"
        )));
    }
    for table in ["jobs", "flow_definitions"] {
        let columns = table_columns(connection, table)?;
        for required in ["period_value", "period_unit", "period_first_at_utc"] {
            if !columns.iter().any(|column| column == required) {
                return Err(StoreError::InvalidData(format!(
                    "schema version {CURRENT_SCHEMA_VERSION} is missing {table}.{required}"
                )));
            }
        }
    }
    let occurrences_sql: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'occurrences'",
        [],
        |row| row.get(0),
    )?;
    if !occurrences_sql.contains("'periodic'") {
        return Err(StoreError::InvalidData(format!(
            "schema version {CURRENT_SCHEMA_VERSION} does not support periodic occurrences"
        )));
    }
    let schedule_events_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schedule_events')",
        [],
        |row| row.get(0),
    )?;
    if !schedule_events_exists {
        return Err(StoreError::InvalidData(format!(
            "schema version {CURRENT_SCHEMA_VERSION} is missing schedule_events"
        )));
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
