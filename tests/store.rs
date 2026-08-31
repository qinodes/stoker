use std::path::PathBuf;

use rusqlite::Connection;
use stoker::{JobState, NewJob, Store, StoreError};
use tempfile::TempDir;

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
