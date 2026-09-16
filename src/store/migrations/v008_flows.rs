use rusqlite::Transaction;

use super::super::error::StoreError;

/// Flow storage is additive. Existing job rows remain the source of truth for
/// the legacy API and are given a serial, retry-zero definition identity.
pub(super) fn apply(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    for (name, definition) in [
        (
            "mode",
            "TEXT NOT NULL DEFAULT 'serial' CHECK (mode IN ('serial', 'scheduled'))",
        ),
        ("schedule_kind", "TEXT"),
        ("schedule_at_utc", "TEXT"),
        ("daily_time", "TEXT"),
        ("schedule_timezone", "TEXT"),
        ("schedule_generation", "INTEGER NOT NULL DEFAULT 1"),
        (
            "enabled",
            "INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))",
        ),
        ("retry", "INTEGER NOT NULL DEFAULT 0 CHECK (retry >= 0)"),
        ("flow_id", "TEXT"),
        ("task_id", "TEXT"),
        ("internal_definition_id", "TEXT"),
        ("last_dispatch_sequence", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !super::table_columns(transaction, "jobs")?
            .iter()
            .any(|column| column == name)
        {
            transaction.execute(
                &format!("ALTER TABLE jobs ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    transaction.execute(
        "UPDATE jobs SET internal_definition_id = id WHERE internal_definition_id IS NULL",
        [],
    )?;
    for (name, definition) in [
        (
            "mode",
            "TEXT NOT NULL DEFAULT 'serial' CHECK (mode IN ('serial', 'scheduled'))",
        ),
        (
            "max_concurrency",
            "INTEGER NOT NULL DEFAULT 2 CHECK (max_concurrency > 0)",
        ),
        ("dispatch_sequence", "INTEGER NOT NULL DEFAULT 0"),
        (
            "recovery_fence",
            "INTEGER NOT NULL DEFAULT 0 CHECK (recovery_fence IN (0, 1))",
        ),
    ] {
        if !super::table_columns(transaction, "settings")?
            .iter()
            .any(|column| column == name)
        {
            transaction.execute(
                &format!("ALTER TABLE settings ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    transaction.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS flow_definitions (
    flow_id TEXT PRIMARY KEY NOT NULL,
    internal_definition_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    owner TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('serial', 'scheduled')),
    schedule_kind TEXT,
    schedule_at_utc TEXT,
    daily_time TEXT,
    schedule_timezone TEXT,
    schedule_generation INTEGER NOT NULL DEFAULT 1,
    schedule_effective_from TEXT,
    committed INTEGER NOT NULL DEFAULT 0 CHECK (committed IN (0, 1)),
    frozen INTEGER NOT NULL DEFAULT 0 CHECK (frozen IN (0, 1)),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    graph_revision INTEGER NOT NULL DEFAULT 0,
    draft_revision INTEGER NOT NULL DEFAULT 0,
    draft_json TEXT,
    queue_order INTEGER,
    created_at TEXT NOT NULL,
    committed_at TEXT,
    last_dispatch_sequence INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS flow_tasks (
    flow_id TEXT NOT NULL REFERENCES flow_definitions(flow_id) ON DELETE CASCADE,
    task_id TEXT NOT NULL,
    name TEXT NOT NULL,
    cwd TEXT NOT NULL,
    command TEXT NOT NULL,
    command_line TEXT NOT NULL,
    retry INTEGER NOT NULL DEFAULT 0 CHECK (retry >= 0),
    depend_mode TEXT NOT NULL DEFAULT 'all' CHECK (depend_mode IN ('all', 'any')),
    sequence INTEGER NOT NULL,
    PRIMARY KEY (flow_id, task_id)
);

CREATE TABLE IF NOT EXISTS flow_dependencies (
    flow_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    upstream_task_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
    PRIMARY KEY (flow_id, task_id, upstream_task_id, status),
    FOREIGN KEY (flow_id, task_id) REFERENCES flow_tasks(flow_id, task_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS occurrences (
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

CREATE TABLE IF NOT EXISTS flow_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    flow_id TEXT NOT NULL,
    internal_definition_id TEXT NOT NULL,
    schedule_generation INTEGER NOT NULL,
    source TEXT NOT NULL,
    state TEXT NOT NULL,
    occurrence_id TEXT,
    definition_snapshot TEXT NOT NULL,
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_runs (
    run_id TEXT NOT NULL REFERENCES flow_runs(run_id) ON DELETE CASCADE,
    task_id TEXT NOT NULL,
    state TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    PRIMARY KEY (run_id, task_id)
);

CREATE TABLE IF NOT EXISTS attempts (
    attempt_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    number INTEGER NOT NULL,
    state TEXT NOT NULL,
    exit_code INTEGER,
    failure_kind TEXT,
    failure_detail TEXT,
    started_at TEXT,
    finished_at TEXT,
    FOREIGN KEY (run_id, task_id) REFERENCES task_runs(run_id, task_id) ON DELETE CASCADE,
    UNIQUE (run_id, task_id, number)
);

CREATE TABLE IF NOT EXISTS manual_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    flow_id TEXT NOT NULL,
    parameters_hash TEXT NOT NULL,
    run_id TEXT,
    result TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS run_edit_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    graph_revision INTEGER NOT NULL,
    task_id TEXT NOT NULL,
    event TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS flow_definitions_mode_queue ON flow_definitions(mode, committed, enabled, queue_order, flow_id);
CREATE INDEX IF NOT EXISTS occurrences_due ON occurrences(state, due_at, flow_id);
CREATE INDEX IF NOT EXISTS flow_runs_flow_state ON flow_runs(flow_id, state, created_at);
CREATE INDEX IF NOT EXISTS task_runs_state ON task_runs(state, next_attempt_at, run_id);
CREATE INDEX IF NOT EXISTS attempts_run_task ON attempts(run_id, task_id, number);
CREATE UNIQUE INDEX IF NOT EXISTS jobs_internal_definition_id ON jobs(internal_definition_id);
CREATE INDEX IF NOT EXISTS jobs_mode_state ON jobs(mode, state, queue_order, id);
        "#,
    )?;
    Ok(())
}
