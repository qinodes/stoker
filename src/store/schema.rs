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
    queue_locked INTEGER NOT NULL DEFAULT 0 CHECK (queue_locked IN (0, 1))
);

INSERT OR IGNORE INTO settings (id, queue_locked) VALUES (1, 0);
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
