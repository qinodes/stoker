use std::path::Path;

use rusqlite::{Connection, params};
use stoker::store::CURRENT_SCHEMA_VERSION;
use stoker::{NewJob, Store, StoreError};
use tempfile::TempDir;
use uuid::Uuid;

const HISTORICAL_FIXTURES: &[(&str, &str, bool, bool)] = &[
    (
        "current_without_command_line",
        include_str!("fixtures/database/current_without_command_line.sql"),
        false,
        true,
    ),
    (
        "current_without_constraints",
        include_str!("fixtures/database/current_without_constraints.sql"),
        false,
        true,
    ),
    (
        "legacy_git_with_queue_order",
        include_str!("fixtures/database/legacy_git_with_queue_order.sql"),
        true,
        true,
    ),
    (
        "legacy_git_without_queue_order",
        include_str!("fixtures/database/legacy_git_without_queue_order.sql"),
        true,
        false,
    ),
];

const VERSIONED_FIXTURES: &[(u32, &str)] = &[
    (1, include_str!("fixtures/database/versioned/v001.sql")),
    (2, include_str!("fixtures/database/versioned/v002.sql")),
    (3, include_str!("fixtures/database/versioned/v003.sql")),
    (4, include_str!("fixtures/database/versioned/v004.sql")),
];

#[test]
fn every_unversioned_fixture_upgrades_preserves_data_and_reopens_idempotently() {
    for &(name, fixture, legacy, had_queue_order) in HISTORICAL_FIXTURES {
        let directory = TempDir::new().unwrap();
        let database = directory.path().join(format!("{name}.db"));
        let id = Uuid::new_v4();
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch(fixture).unwrap();
        insert_fixture_job(
            &connection,
            directory.path(),
            id,
            name,
            legacy,
            had_queue_order,
        );
        drop(connection);

        let store = Store::open(&database).unwrap();
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        let migrated = store.get_job(id).unwrap();
        assert_eq!(migrated.name, name);
        assert_eq!(
            migrated.queue_order,
            Some(if had_queue_order { 7 } else { 1 })
        );
        if legacy {
            assert!(migrated.cwd.is_absolute());
            assert!(migrated.cwd.ends_with("work"));
        }
        drop(store);

        let reopened = Store::open(&database).unwrap();
        assert_eq!(reopened.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(reopened.get_job(id).unwrap(), migrated);
        assert_latest_schema(&database);
    }
}

#[test]
fn every_numbered_schema_fixture_upgrades_directly_to_current() {
    for &(version, fixture) in VERSIONED_FIXTURES {
        let directory = TempDir::new().unwrap();
        let database = directory.path().join(format!("v{version:03}.db"));
        Connection::open(&database)
            .unwrap()
            .execute_batch(fixture)
            .unwrap();

        let store = Store::open(&database).unwrap();
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        let id = Uuid::parse_str(&format!("00000000-0000-0000-0000-{version:012}")).unwrap();
        let job = store.get_job(id).unwrap();
        assert_eq!(job.name, format!("v{version:03}"));
        if version == 4 {
            assert_eq!(job.description.as_deref(), Some("preserved"));
        }
        drop(store);
        assert_latest_schema(&database);
    }
}

#[test]
fn failed_migration_rolls_back_its_schema_and_can_resume_after_data_is_fixed() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("rollback.db");
    let id = Uuid::new_v4();
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!(
            "fixtures/database/current_without_constraints.sql"
        ))
        .unwrap();
    connection
        .execute(
            "INSERT INTO jobs
             (id,name,user,cwd,command,command_line,state,queue_order,created_at)
             VALUES (?1,?2,'alice','/tmp','[]',NULL,'QUEUED',1,'2026-01-01T00:00:00Z')",
            params![id.to_string(), "x".repeat(129)],
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        Store::open(&database),
        Err(StoreError::InvalidData(_))
    ));
    let connection = Connection::open(&database).unwrap();
    assert_eq!(schema_version(&connection), 4);
    assert!(!table_exists(&connection, "jobs_v5"));
    assert!(table_exists(&connection, "jobs"));
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM jobs WHERE id = ?1",
                [id.to_string()],
                |row| { row.get::<_, String>(0) }
            )
            .unwrap()
            .len(),
        129
    );
    connection
        .execute(
            "UPDATE jobs SET name = 'recovered' WHERE id = ?1",
            [id.to_string()],
        )
        .unwrap();
    drop(connection);

    let store = Store::open(&database).unwrap();
    assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert_eq!(store.get_job(id).unwrap().name, "recovered");
}

#[test]
fn newer_schema_versions_are_rejected_without_mutation() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("future.db");
    let connection = Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
        .unwrap();
    drop(connection);

    let error = match Store::open(&database) {
        Ok(_) => panic!("future schema unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::InvalidData(_)));
    let connection = Connection::open(&database).unwrap();
    assert_eq!(schema_version(&connection), CURRENT_SCHEMA_VERSION + 1);
    assert!(!table_exists(&connection, "jobs"));
}

#[test]
fn a_database_claiming_the_latest_version_must_match_the_latest_schema() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("false-latest.db");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!(
            "fixtures/database/current_without_constraints.sql"
        ))
        .unwrap();
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
        .unwrap();
    drop(connection);

    let error = Store::open(&database).err().unwrap();
    assert!(
        matches!(error, StoreError::InvalidData(ref message) if message.contains("missing jobs.description"))
    );
    assert_eq!(
        schema_version(&Connection::open(&database).unwrap()),
        CURRENT_SCHEMA_VERSION
    );

    for (name, mutation, expected) in [
        ("settings", "DROP TABLE settings", "missing settings"),
        (
            "settings-row",
            "DELETE FROM settings WHERE id = 1",
            "missing settings row 1",
        ),
        (
            "index",
            "DROP INDEX jobs_state_queue_order_id",
            "missing index jobs_state_queue_order_id",
        ),
    ] {
        let database = directory.path().join(format!("false-latest-{name}.db"));
        drop(Store::open(&database).unwrap());
        Connection::open(&database)
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        let error = Store::open(&database).err().unwrap();
        assert!(
            matches!(error, StoreError::InvalidData(ref message) if message.contains(expected))
        );
    }

    let database = directory.path().join("false-latest-constraint.db");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("fixtures/database/versioned/v004.sql"))
        .unwrap();
    connection
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
        .unwrap();
    drop(connection);
    let error = Store::open(&database).err().unwrap();
    assert!(
        matches!(error, StoreError::InvalidData(ref message) if message.contains("missing jobs constraint"))
    );
}

#[test]
fn cloned_store_handles_share_one_reusable_connection() {
    let directory = TempDir::new().unwrap();
    let store = Store::open(directory.path().join("shared.db")).unwrap();
    let cloned = store.clone();
    let id = store
        .create_job(NewJob {
            name: "shared".into(),
            user: "alice".into(),
            description: None,
            cwd: directory.path().to_path_buf(),
            command: vec!["echo".into(), "ok".into()],
        })
        .unwrap();
    assert_eq!(cloned.get_job(id).unwrap().name, "shared");
}

#[test]
fn v10_periodic_migration_preserves_existing_occurrences() {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("v009-periodic-upgrade.db");
    drop(Store::open(&database).unwrap());
    let occurrence_id = Uuid::new_v4();
    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "INSERT INTO occurrences (occurrence_id, internal_definition_id, flow_id, schedule_generation, kind, occurrence_key, local_date, due_at, state, reason, created_at) VALUES (?1, 'definition', 'flow', 1, 'once', 'once', NULL, '2099-01-01T00:00:00+00:00', 'PENDING', NULL, '2026-01-01T00:00:00+00:00')",
            [occurrence_id.to_string()],
        )
        .unwrap();
    connection
        .execute_batch(
            r#"
DROP INDEX occurrences_due;
ALTER TABLE occurrences RENAME TO occurrences_v10;
CREATE TABLE occurrences (
    occurrence_id TEXT PRIMARY KEY NOT NULL,
    internal_definition_id TEXT NOT NULL,
    flow_id TEXT NOT NULL,
    schedule_generation INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('once', 'daily')),
    occurrence_key TEXT NOT NULL,
    local_date TEXT,
    due_at TEXT NOT NULL,
    state TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (internal_definition_id, schedule_generation, kind, occurrence_key)
);
INSERT INTO occurrences SELECT * FROM occurrences_v10;
DROP TABLE occurrences_v10;
CREATE INDEX occurrences_due ON occurrences(state, due_at, flow_id);
ALTER TABLE jobs DROP COLUMN period_value;
ALTER TABLE jobs DROP COLUMN period_unit;
ALTER TABLE jobs DROP COLUMN period_first_at_utc;
ALTER TABLE flow_definitions DROP COLUMN period_value;
ALTER TABLE flow_definitions DROP COLUMN period_unit;
ALTER TABLE flow_definitions DROP COLUMN period_first_at_utc;
PRAGMA user_version = 9;
            "#,
        )
        .unwrap();
    drop(connection);

    let store = Store::open(&database).unwrap();
    assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    drop(store);
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM occurrences WHERE occurrence_id = ?1",
                [occurrence_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "PENDING"
    );
    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'occurrences'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sql.contains("'periodic'"));
}

fn insert_fixture_job(
    connection: &Connection,
    root: &Path,
    id: Uuid,
    name: &str,
    legacy: bool,
    had_queue_order: bool,
) {
    let queue_column = if had_queue_order { ",queue_order" } else { "" };
    let queue_value = if had_queue_order { ",7" } else { "" };
    if legacy {
        connection
            .execute(
                &format!(
                    "INSERT INTO jobs
                     (id,name,user,repository,git_commit,cwd,command,state{queue_column},created_at,committed_at)
                     VALUES (?1,?2,'alice',?3,'commit','work','[]','QUEUED'{queue_value},?4,?4)"
                ),
                params![
                    id.to_string(),
                    name,
                    root.to_string_lossy(),
                    "2026-01-01T00:00:00Z"
                ],
            )
            .unwrap();
    } else {
        connection
            .execute(
                "INSERT INTO jobs
                 (id,name,user,cwd,command,state,queue_order,created_at,committed_at)
                 VALUES (?1,?2,'alice','/tmp','[]','QUEUED',7,?3,?3)",
                params![id.to_string(), name, "2026-01-01T00:00:00Z"],
            )
            .unwrap();
    }
}

fn assert_latest_schema(database: &Path) {
    let connection = Connection::open(database).unwrap();
    assert_eq!(schema_version(&connection), CURRENT_SCHEMA_VERSION);
    let columns: Vec<String> = connection
        .prepare("PRAGMA table_info(jobs)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for required in [
        "command_line",
        "queue_order",
        "description",
        "description_revision",
        "period_value",
        "period_unit",
        "period_first_at_utc",
    ] {
        assert!(columns.iter().any(|column| column == required));
    }
    for index in ["jobs_state_queue_order_id", "jobs_user_state_created_at_id"] {
        assert!(table_exists_with_type(&connection, "index", index));
    }
    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'jobs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sql.contains("length(name) <= 128"));
    assert!(sql.contains("length(user) <= 50"));
    assert!(sql.contains("length(description) <= 200"));
}

fn schema_version(connection: &Connection) -> u32 {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn table_exists(connection: &Connection, name: &str) -> bool {
    table_exists_with_type(connection, "table", name)
}

fn table_exists_with_type(connection: &Connection, object_type: &str, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
            [object_type, name],
            |row| row.get(0),
        )
        .unwrap()
}
