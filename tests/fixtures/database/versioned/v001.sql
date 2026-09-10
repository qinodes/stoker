CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    user TEXT NOT NULL,
    cwd TEXT NOT NULL,
    command TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    committed_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    exit_code INTEGER,
    pid INTEGER,
    failure_detail TEXT
);
CREATE TABLE settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    queue_locked INTEGER NOT NULL DEFAULT 0 CHECK (queue_locked IN (0, 1))
);
INSERT INTO settings (id, queue_locked) VALUES (1, 0);
INSERT INTO jobs (id,name,user,cwd,command,state,created_at)
VALUES ('00000000-0000-0000-0000-000000000001','v001','alice','/tmp','[]','DRAFT','2026-01-01T00:00:00Z');
PRAGMA user_version = 1;
