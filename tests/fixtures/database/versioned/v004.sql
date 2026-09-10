CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    user TEXT NOT NULL,
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
    description TEXT,
    description_revision INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    queue_locked INTEGER NOT NULL DEFAULT 0 CHECK (queue_locked IN (0, 1))
);
INSERT INTO settings (id, queue_locked) VALUES (1, 0);
CREATE INDEX jobs_state_queue_order_id ON jobs (state, queue_order, id);
CREATE INDEX jobs_user_state_created_at_id ON jobs (user, state, created_at, id);
INSERT INTO jobs (id,name,user,cwd,command,state,created_at,description)
VALUES ('00000000-0000-0000-0000-000000000004','v004','alice','/tmp','[]','DRAFT','2026-01-01T00:00:00Z','preserved');
PRAGMA user_version = 4;
