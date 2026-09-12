pub(super) const BASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL CHECK (length(name) <= 128),
    user TEXT NOT NULL CHECK (length(user) <= 50),
    cwd TEXT NOT NULL,
    command TEXT NOT NULL,
    command_line TEXT,
    state TEXT NOT NULL,
    queue_order INTEGER,
    created_at TEXT NOT NULL,
    committed_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    exit_code INTEGER,
    pid INTEGER,
    failure_detail TEXT,
    description TEXT CHECK (description IS NULL OR length(description) <= 200),
    description_revision INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    queue_locked INTEGER NOT NULL DEFAULT 0 CHECK (queue_locked IN (0, 1)),
    log_max_bytes_per_job INTEGER NOT NULL DEFAULT 67108864,
    log_segment_bytes INTEGER NOT NULL DEFAULT 1048576,
    log_max_bytes_total INTEGER NOT NULL DEFAULT 1073741824,
    log_retention_jobs INTEGER NOT NULL DEFAULT 100,
    log_disk_reserve_bytes INTEGER NOT NULL DEFAULT 536870912,
    termination_grace_ms INTEGER NOT NULL DEFAULT 500,
    max_runtime_ms INTEGER,
    startup_timeout_ms INTEGER NOT NULL DEFAULT 30000
);

INSERT OR IGNORE INTO settings
    (id, queue_locked, log_max_bytes_per_job, log_segment_bytes,
     log_max_bytes_total, log_retention_jobs, log_disk_reserve_bytes)
VALUES (1, 0, 67108864, 1048576, 1073741824, 100, 536870912);
"#;

pub(super) const INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS jobs_state_queue_order_id
    ON jobs (state, queue_order, id);
CREATE INDEX IF NOT EXISTS jobs_user_state_created_at_id
    ON jobs (user, state, created_at, id);
"#;

pub(super) const JOB_COLUMNS: &str =
    "id,name,user,cwd,command,command_line,state,queue_order,created_at,
     committed_at,started_at,finished_at,exit_code,pid,failure_detail,
     description,description_revision";
