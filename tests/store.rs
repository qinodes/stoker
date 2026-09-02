use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use rusqlite::{Connection, params};
use stoker::{JobState, NewJob, Store, StoreError};
use tempfile::TempDir;
use uuid::Uuid;

fn test_store() -> Store {
    let dir = TempDir::new().unwrap();
    // Keep the directory alive for the duration of this test process. The store
    // owns the open database, so leaking this small test fixture is intentional.
    let path = Box::leak(Box::new(dir)).path().join("stoker.db");
    Store::open(path).unwrap()
}

fn new_job() -> NewJob {
    NewJob {
        name: "example".into(),
        user: "alice".into(),
        repository: PathBuf::from("/tmp/repository"),
        git_commit: "0123456789abcdef".into(),
        cwd: PathBuf::from("experiments/example"),
        command: vec!["echo".into(), "ok".into()],
    }
}

fn new_job_named(name: &str) -> NewJob {
    NewJob {
        name: name.into(),
        ..new_job()
    }
}

#[test]
fn commit_moves_draft_to_queued_once() {
    let store = test_store();
    let id = store.create_job(new_job()).unwrap();
    assert_eq!(store.commit_job(id).unwrap().state, JobState::Queued);
    assert!(matches!(
        store.commit_job(id),
        Err(StoreError::InvalidTransition { .. })
    ));
}

#[test]
fn claim_next_returns_committed_jobs_in_commit_order() {
    let store = test_store();
    let first = store.create_job(new_job_named("first")).unwrap();
    let second = store.create_job(new_job_named("second")).unwrap();
    store.commit_job(first).unwrap();
    store.commit_job(second).unwrap();
    assert_eq!(store.claim_next().unwrap().unwrap().id, first);
    assert_eq!(store.claim_next().unwrap().unwrap().id, second);
    assert!(store.claim_next().unwrap().is_none());
}

#[test]
fn queued_jobs_have_contiguous_orders_after_cancel_and_claim() {
    let store = test_store();
    let first = store.create_job(new_job_named("first")).unwrap();
    let second = store.create_job(new_job_named("second")).unwrap();
    let third = store.create_job(new_job_named("third")).unwrap();
    assert_eq!(store.commit_job(first).unwrap().queue_order, Some(1));
    assert_eq!(store.commit_job(second).unwrap().queue_order, Some(2));
    assert_eq!(store.commit_job(third).unwrap().queue_order, Some(3));

    assert_eq!(store.cancel_not_started(second).unwrap().queue_order, None);
    assert_eq!(store.get_job(first).unwrap().queue_order, Some(1));
    assert_eq!(store.get_job(third).unwrap().queue_order, Some(2));

    assert_eq!(store.claim_next().unwrap().unwrap().id, first);
    assert_eq!(store.get_job(first).unwrap().queue_order, None);
    assert_eq!(store.get_job(third).unwrap().queue_order, Some(1));
}

#[test]
fn legacy_database_migrates_queued_jobs_to_queue_order() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    {
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE jobs (
                    id TEXT PRIMARY KEY NOT NULL,
                    name TEXT NOT NULL,
                    user TEXT NOT NULL,
                    repository TEXT NOT NULL,
                    git_commit TEXT NOT NULL,
                    cwd TEXT NOT NULL,
                    command TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    committed_at TEXT,
                    started_at TEXT,
                    finished_at TEXT,
                    exit_code INTEGER,
                    pid INTEGER,
                    execution_dir TEXT,
                    failure_detail TEXT
                )",
            )
            .unwrap();
        for (id, name, committed_at) in [
            (first, "first", "2026-01-01T00:00:00+00:00"),
            (second, "second", "2026-01-01T00:01:00+00:00"),
        ] {
            connection
                .execute(
                    "INSERT INTO jobs (id,name,user,repository,git_commit,cwd,command,state,created_at,committed_at)
                     VALUES (?1,?2,'alice','/tmp/repo','commit','.','[\"echo\"]','QUEUED',?3,?3)",
                    params![id.to_string(), name, committed_at],
                )
                .unwrap();
        }
    }

    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.get_job(first).unwrap().queue_order, Some(1));
    assert_eq!(store.get_job(second).unwrap().queue_order, Some(2));
}

#[test]
fn concurrent_commits_assign_unique_contiguous_orders() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let first_store = Arc::new(Store::open(&db_path).unwrap());
    let second_store = Arc::new(Store::open(&db_path).unwrap());
    let first = first_store.create_job(new_job_named("first")).unwrap();
    let second = first_store.create_job(new_job_named("second")).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let first_barrier = Arc::clone(&barrier);
    let first_handle = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.commit_job(first).unwrap();
    });
    let second_barrier = Arc::clone(&barrier);
    let second_handle = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.commit_job(second).unwrap();
    });
    barrier.wait();
    first_handle.join().unwrap();
    second_handle.join().unwrap();

    let store = Store::open(&db_path).unwrap();
    let orders: Vec<_> = store
        .list_jobs_with_state(None, Some(JobState::Queued))
        .unwrap()
        .into_iter()
        .map(|job| job.queue_order)
        .collect();
    assert_eq!(orders, vec![Some(1), Some(2)]);
}

#[test]
fn concurrent_claim_and_commit_leave_the_new_job_first_in_queue() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let claim_store = Arc::new(Store::open(&db_path).unwrap());
    let commit_store = Arc::new(Store::open(&db_path).unwrap());
    let claimed = claim_store.create_job(new_job_named("claimed")).unwrap();
    let appended = claim_store.create_job(new_job_named("appended")).unwrap();
    claim_store.commit_job(claimed).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let claim_barrier = Arc::clone(&barrier);
    let claim_handle = std::thread::spawn(move || {
        claim_barrier.wait();
        claim_store.claim_next().unwrap().unwrap();
    });
    let commit_barrier = Arc::clone(&barrier);
    let commit_handle = std::thread::spawn(move || {
        commit_barrier.wait();
        commit_store.commit_job(appended).unwrap();
    });
    barrier.wait();
    claim_handle.join().unwrap();
    commit_handle.join().unwrap();

    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.get_job(claimed).unwrap().state, JobState::Starting);
    assert_eq!(store.get_job(appended).unwrap().queue_order, Some(1));
}

#[test]
fn concurrent_cancel_and_commit_leave_the_new_job_first_in_queue() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let cancel_store = Arc::new(Store::open(&db_path).unwrap());
    let commit_store = Arc::new(Store::open(&db_path).unwrap());
    let cancelled = cancel_store.create_job(new_job_named("cancelled")).unwrap();
    let appended = cancel_store.create_job(new_job_named("appended")).unwrap();
    cancel_store.commit_job(cancelled).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let cancel_barrier = Arc::clone(&barrier);
    let cancel_handle = std::thread::spawn(move || {
        cancel_barrier.wait();
        cancel_store.cancel_not_started(cancelled).unwrap();
    });
    let commit_barrier = Arc::clone(&barrier);
    let commit_handle = std::thread::spawn(move || {
        commit_barrier.wait();
        commit_store.commit_job(appended).unwrap();
    });
    barrier.wait();
    cancel_handle.join().unwrap();
    commit_handle.join().unwrap();

    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.get_job(cancelled).unwrap().state, JobState::Cancelled);
    assert_eq!(store.get_job(appended).unwrap().queue_order, Some(1));
}

#[test]
fn restart_marks_only_runtime_states_lost() {
    let dir = Box::leak(Box::new(TempDir::new().unwrap()));
    let db_path = dir.path().join("stoker.db");
    let store = Store::open(&db_path).unwrap();
    let starting = store.create_job(new_job_named("starting")).unwrap();
    let running = store.create_job(new_job_named("running")).unwrap();
    let cancelling = store.create_job(new_job_named("cancelling")).unwrap();
    let queued = store.create_job(new_job_named("queued")).unwrap();
    let connection = Connection::open(&db_path).unwrap();
    for (id, state) in [
        (starting, "STARTING"),
        (running, "RUNNING"),
        (cancelling, "CANCELLING"),
        (queued, "QUEUED"),
    ] {
        connection
            .execute(
                "UPDATE jobs SET state = ?2 WHERE id = ?1",
                (id.to_string(), state),
            )
            .unwrap();
    }
    store.mark_runtime_jobs_lost().unwrap();
    assert_eq!(store.get_job(starting).unwrap().state, JobState::Lost);
    assert_eq!(store.get_job(running).unwrap().state, JobState::Lost);
    assert_eq!(store.get_job(cancelling).unwrap().state, JobState::Lost);
    assert_eq!(store.get_job(queued).unwrap().state, JobState::Queued);
}
