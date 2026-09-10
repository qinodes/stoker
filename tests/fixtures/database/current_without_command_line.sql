CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    user TEXT NOT NULL,
    cwd TEXT NOT NULL,
    command TEXT NOT NULL,
    state TEXT NOT NULL,
    queue_order INTEGER,
    created_at TEXT NOT NULL,
    committed_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    exit_code INTEGER,
    pid INTEGER,
    failure_detail TEXT
);
