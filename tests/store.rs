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
        cwd: PathBuf::from("/tmp/repository/experiments/example"),
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
fn queue_lock_is_durable_idempotent_and_blocks_claims() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let store = Store::open(&db_path).unwrap();
    assert!(!store.queue_locked().unwrap());

    let id = store.create_job(new_job()).unwrap();
    store.commit_job(id).unwrap();
    let queued_before_lock = store.get_job(id).unwrap();

    store.lock_queue().unwrap();
    store.lock_queue().unwrap();
    assert!(store.queue_locked().unwrap());
    drop(store);

    let store = Store::open(&db_path).unwrap();
    assert!(store.queue_locked().unwrap());
    assert!(store.claim_next().unwrap().is_none());
    assert_eq!(store.get_job(id).unwrap(), queued_before_lock);

    store.unlock_queue().unwrap();
    store.unlock_queue().unwrap();
    assert!(!store.queue_locked().unwrap());
    assert_eq!(store.claim_next().unwrap().unwrap().id, id);
}

#[test]
fn queue_lock_rejects_queue_mutations_but_allows_cancellation() {
    let store = test_store();
    let draft = store.create_job(new_job_named("draft")).unwrap();
    let queued = store.create_job(new_job_named("queued")).unwrap();
    store.commit_job(queued).unwrap();
    store.lock_queue().unwrap();

    let commit_error = store.commit_job(draft).unwrap_err();
    assert!(matches!(&commit_error, StoreError::QueueLocked));
    assert!(commit_error.to_string().contains("stoker queue unlock"));
    assert!(matches!(
        store.commit_all_drafts(),
        Err(StoreError::QueueLocked)
    ));
    assert_eq!(
        store.cancel_not_started(queued).unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn moving_a_queued_job_rewrites_contiguous_order_and_validates_destination() {
    let store = test_store();
    let first = store.create_job(new_job_named("first")).unwrap();
    let second = store.create_job(new_job_named("second")).unwrap();
    let third = store.create_job(new_job_named("third")).unwrap();
    for id in [first, second, third] {
        store.commit_job(id).unwrap();
    }
    store.lock_queue().unwrap();

    let moved = store.move_queued_job(third, 1).unwrap();
    assert_eq!(
        moved.iter().map(|job| job.id).collect::<Vec<_>>(),
        [third, first, second]
    );
    assert_eq!(
        moved.iter().map(|job| job.queue_order).collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3)]
    );

    for target_order in [0, 4] {
        let error = store.move_queued_job(first, target_order).unwrap_err();
        assert!(matches!(error, StoreError::InvalidQueueOrder { .. }));
        assert_eq!(
            store
                .list_jobs_with_state(None, Some(JobState::Queued))
                .unwrap()
                .iter()
                .map(|job| job.id)
                .collect::<Vec<_>>(),
            [third, first, second]
        );
    }
}

#[test]
fn moving_a_queued_job_races_with_cancellation_without_restoring_it() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let move_store = Arc::new(Store::open(&db_path).unwrap());
    let cancel_store = Arc::new(Store::open(&db_path).unwrap());
    let first = move_store.create_job(new_job_named("first")).unwrap();
    let second = move_store.create_job(new_job_named("second")).unwrap();
    let selected = move_store.create_job(new_job_named("selected")).unwrap();
    for id in [first, second, selected] {
        move_store.commit_job(id).unwrap();
    }
    move_store.lock_queue().unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let move_barrier = Arc::clone(&barrier);
    let move_handle = std::thread::spawn(move || {
        move_barrier.wait();
        move_store.move_queued_job(selected, 1)
    });
    let cancel_barrier = Arc::clone(&barrier);
    let cancel_handle = std::thread::spawn(move || {
        cancel_barrier.wait();
        cancel_store.cancel_not_started(selected)
    });
    barrier.wait();
    let move_result = move_handle.join().unwrap();
    let cancel_result = cancel_handle.join().unwrap();
    assert!(matches!(
        move_result,
        Ok(_) | Err(StoreError::InvalidTransition { .. })
    ));
    assert_eq!(cancel_result.unwrap().state, JobState::Cancelled);

    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.get_job(selected).unwrap().state, JobState::Cancelled);
    assert_eq!(store.get_job(selected).unwrap().queue_order, None);
    let queued = store
        .list_jobs_with_state(None, Some(JobState::Queued))
        .unwrap();
    assert_eq!(
        queued.iter().map(|job| job.id).collect::<Vec<_>>(),
        [first, second]
    );
    assert_eq!(
        queued.iter().map(|job| job.queue_order).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
}

#[test]
fn moving_after_cancelling_a_different_job_uses_current_queue_order() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let move_store = Arc::new(Store::open(&db_path).unwrap());
    let cancel_store = Arc::new(Store::open(&db_path).unwrap());
    let cancelled = move_store.create_job(new_job_named("cancelled")).unwrap();
    let second = move_store.create_job(new_job_named("second")).unwrap();
    let selected = move_store.create_job(new_job_named("selected")).unwrap();
    for id in [cancelled, second, selected] {
        move_store.commit_job(id).unwrap();
    }
    move_store.lock_queue().unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let move_barrier = Arc::clone(&barrier);
    let move_handle = std::thread::spawn(move || {
        move_barrier.wait();
        move_store.move_queued_job(selected, 1)
    });
    let cancel_barrier = Arc::clone(&barrier);
    let cancel_handle = std::thread::spawn(move || {
        cancel_barrier.wait();
        cancel_store.cancel_not_started(cancelled)
    });
    barrier.wait();

    assert!(move_handle.join().unwrap().is_ok());
    assert_eq!(
        cancel_handle.join().unwrap().unwrap().state,
        JobState::Cancelled
    );

    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.get_job(cancelled).unwrap().state, JobState::Cancelled);
    assert_eq!(store.get_job(cancelled).unwrap().queue_order, None);
    let queued = store
        .list_jobs_with_state(None, Some(JobState::Queued))
        .unwrap();
    assert_eq!(
        queued.iter().map(|job| job.id).collect::<Vec<_>>(),
        [selected, second]
    );
    assert_eq!(
        queued.iter().map(|job| job.queue_order).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
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
fn commit_all_drafts_uses_creation_time_order() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    let store = Store::open(&db_path).unwrap();
    let first = store.create_job(new_job_named("first")).unwrap();
    let second = store.create_job(new_job_named("second")).unwrap();
    let third = store.create_job(new_job_named("third")).unwrap();
    let connection = Connection::open(&db_path).unwrap();
    for (id, created_at) in [
        (first, "2026-01-01T00:02:00+00:00"),
        (second, "2026-01-01T00:01:00+00:00"),
        (third, "2026-01-01T00:00:00+00:00"),
    ] {
        connection
            .execute(
                "UPDATE jobs SET created_at = ?2 WHERE id = ?1",
                params![id.to_string(), created_at],
            )
            .unwrap();
    }

    let committed = store.commit_all_drafts().unwrap();
    assert_eq!(
        committed.iter().map(|job| job.id).collect::<Vec<_>>(),
        [third, second, first]
    );
    assert_eq!(
        committed
            .iter()
            .map(|job| job.queue_order)
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3)]
    );
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
        let repository = dir.path().join("repo");
        for (id, name, cwd, committed_at) in [
            (
                first,
                "first",
                "experiments/first",
                "2026-01-01T00:00:00+00:00",
            ),
            (
                second,
                "second",
                "experiments/second",
                "2026-01-01T00:01:00+00:00",
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO jobs (id,name,user,repository,git_commit,cwd,command,state,created_at,committed_at)
                     VALUES (?1,?2,'alice',?3,'commit',?4,'[\"echo\"]','QUEUED',?5,?5)",
                    params![
                        id.to_string(),
                        name,
                        repository.to_string_lossy().to_string(),
                        cwd,
                        committed_at,
                    ],
                )
                .unwrap();
        }
    }

    let store = Store::open(&db_path).unwrap();
    let first_job = store.get_job(first).unwrap();
    let second_job = store.get_job(second).unwrap();
    assert_eq!(first_job.queue_order, Some(1));
    assert_eq!(second_job.queue_order, Some(2));
    assert!(first_job.cwd.is_absolute());
    assert!(second_job.cwd.is_absolute());
    assert!(
        first_job
            .cwd
            .ends_with(PathBuf::from("repo").join("experiments/first"))
    );
    assert!(
        second_job
            .cwd
            .ends_with(PathBuf::from("repo").join("experiments/second"))
    );
    assert_eq!(first_job.command_line, None);
    assert_eq!(second_job.command_line, None);
}

#[test]
fn current_database_without_command_line_gets_migrated() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("stoker.db");
    Connection::open(&db_path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE jobs (
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
            )",
        )
        .unwrap();

    let store = Store::open(&db_path).unwrap();
    let columns: Vec<String> = Connection::open(&db_path)
        .unwrap()
        .prepare("PRAGMA table_info(jobs)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(columns.iter().any(|column| column == "command_line"));
    let id = store.create_job(new_job()).unwrap();
    assert_eq!(store.get_job(id).unwrap().command_line, None);
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

#[test]
fn clean_terminal_jobs_removes_all_terminal_states_but_keeps_active_and_pending() {
    let store = test_store();
    let succeeded = store.create_job(new_job_named("succeeded")).unwrap();
    store.commit_job(succeeded).unwrap();
    store.claim_next().unwrap().unwrap();
    store.set_running(succeeded, 1).unwrap();
    store.finish(succeeded, Some(0), None).unwrap();

    let failed = store.create_job(new_job_named("failed")).unwrap();
    store.commit_job(failed).unwrap();
    store.claim_next().unwrap().unwrap();
    store.set_running(failed, 1).unwrap();
    store.finish(failed, Some(1), Some("failed")).unwrap();

    let cancelled = store.create_job(new_job_named("cancelled")).unwrap();
    store.cancel_not_started(cancelled).unwrap();

    let lost = store.create_job(new_job_named("lost")).unwrap();
    store.commit_job(lost).unwrap();
    store.claim_next().unwrap().unwrap();
    store.mark_runtime_jobs_lost().unwrap();

    let draft = store.create_job(new_job_named("draft")).unwrap();
    let running = store.create_job(new_job_named("running")).unwrap();
    store.commit_job(running).unwrap();
    store.claim_next().unwrap().unwrap();
    store.set_running(running, 1).unwrap();
    let queued = store.create_job(new_job_named("queued")).unwrap();
    store.commit_job(queued).unwrap();

    let removed = store.clean_terminal_jobs().unwrap();
    assert_eq!(removed.len(), 4);
    for state in [
        JobState::Succeeded,
        JobState::Failed,
        JobState::Cancelled,
        JobState::Lost,
    ] {
        assert!(removed.iter().any(|job| job.state == state));
    }
    assert!(matches!(
        store.get_job(succeeded),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_job(failed),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_job(cancelled),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_job(lost),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(store.get_job(draft).unwrap().state, JobState::Draft);
    assert_eq!(store.get_job(queued).unwrap().state, JobState::Queued);
    assert_eq!(store.get_job(running).unwrap().state, JobState::Running);
}
