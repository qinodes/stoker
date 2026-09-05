mod support;

use predicates::prelude::*;
use rusqlite::Connection;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use stoker::{JobState, NewJob, ServiceClient, ServiceStatus, StokerPaths, Store};
use support::{TempStokerHome, stoker_with_home};
use tokio::runtime::Runtime;
use tokio::sync::Barrier;

fn start_service(home: &TempStokerHome) {
    let status = Command::new(assert_cmd::cargo::cargo_bin("stoker"))
        .arg("start")
        .env("STOKER_HOME", home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "failed to start scheduler service: {status}"
    );
}

fn wait_for_state(store: &Store, id: uuid::Uuid, expected: JobState) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.get_job(id).unwrap().state != expected {
        assert!(
            Instant::now() < deadline,
            "job {id} did not reach {expected}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn service_paths(home: &TempStokerHome) -> StokerPaths {
    let root = home.path().to_path_buf();
    StokerPaths {
        database: root.join("stoker.db"),
        runs: root.join("runs"),
        lock: root.join("stoker.lock"),
        endpoint: root.join("stoker.sock"),
        root,
    }
}

struct ServiceCleanup {
    home: PathBuf,
}

impl Drop for ServiceCleanup {
    fn drop(&mut self) {
        let _ = stoker_with_home(&self.home)
            .args(["stop", "--yes"])
            .output();
    }
}

fn wait_for_service_status<F>(
    runtime: &Runtime,
    client: &ServiceClient,
    description: &str,
    predicate: F,
) -> ServiceStatus
where
    F: Fn(&ServiceStatus) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(status) = runtime.block_on(client.status())
            && predicate(&status)
        {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn long_running_job(name: &str, cwd: PathBuf) -> NewJob {
    NewJob {
        name: name.into(),
        user: "test".into(),
        cwd,
        command: if cfg!(windows) {
            vec![
                "cmd".into(),
                "/C".into(),
                "ping 127.0.0.1 -n 31 > NUL".into(),
            ]
        } else {
            vec!["sh".into(), "-c".into(), "sleep 30".into()]
        },
    }
}

fn queued_job(name: &str, cwd: PathBuf) -> NewJob {
    NewJob {
        name: name.into(),
        user: "test".into(),
        cwd,
        command: if cfg!(windows) {
            vec!["cmd".into(), "/C".into(), "echo queued".into()]
        } else {
            vec!["sh".into(), "-c".into(), "printf queued".into()]
        },
    }
}

#[test]
fn start_detaches_and_status_reports_running_service() {
    let home = TempStokerHome::new();
    start_service(&home);
    stoker_with_home(&home)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scheduler: running"));
    stoker_with_home(&home)
        .args(["stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scheduler stopped."));
}

#[test]
fn stop_when_service_is_not_running_is_successful() {
    stoker_with_home(TempStokerHome::new())
        .args(["stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scheduler is not running."));
}

#[test]
fn commit_when_service_is_not_running_reports_how_to_start_it() {
    let home = TempStokerHome::new();
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let id = store
        .create_job(queued_job("offline-commit", cwd.path().to_path_buf()))
        .unwrap();

    stoker_with_home(&home)
        .args(["commit", &id.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Scheduler is not running. Run `stoker start` first.",
        ));
}

#[test]
fn single_commit_reports_the_queued_state() {
    let home = TempStokerHome::new();
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let id = store
        .create_job(queued_job("commit-feedback", cwd.path().to_path_buf()))
        .unwrap();
    start_service(&home);

    stoker_with_home(&home)
        .args(["commit", &id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Committed job {id} (QUEUED)."
        )));

    // The assertion above only waits for the commit acknowledgement. On
    // slower platforms the short-lived job may still be starting when the
    // cleanup stop request arrives, making the shutdown response exceed the
    // IPC timeout. This test is about commit feedback, so wait for execution
    // to finish before tearing down the service.
    wait_for_state(&store, id, JobState::Succeeded);
    stoker_with_home(&home)
        .args(["stop", "--yes"])
        .assert()
        .success();
}

#[test]
fn queue_lock_stopped_service_is_idempotent_and_status_reports_state() {
    let home = TempStokerHome::new();

    stoker_with_home(&home)
        .args(["queue", "lock"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Queue locked."));
    stoker_with_home(&home)
        .args(["queue", "lock"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Queue already locked."));
    stoker_with_home(&home)
        .args(["status"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Scheduler: stopped")
                .and(predicate::str::contains("Queue: locked"))
                .and(predicate::str::contains(
                    "will not start another queued job",
                )),
        );

    stoker_with_home(&home)
        .args(["queue", "unlock"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Queue unlocked."));
    stoker_with_home(&home)
        .args(["queue", "unlock"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Queue already unlocked."));
    stoker_with_home(&home)
        .args(["status"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Scheduler: stopped")
                .and(predicate::str::contains("Queue: unlocked")),
        );
}

#[test]
fn locking_empty_queue_reports_that_there_is_nothing_to_reorder() {
    let home = TempStokerHome::new();
    stoker_with_home(&home)
        .args(["queue", "lock"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Queue locked. No queued jobs to reorder.",
        ));
    stoker_with_home(&home)
        .args(["queue", "edit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No queued jobs to reorder."));
}

#[test]
fn queue_edit_requires_a_locked_queue() {
    stoker_with_home(TempStokerHome::new())
        .args(["queue", "edit"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Queue is unlocked. Run 'stoker queue lock' first.",
        ));
}

#[test]
fn online_locked_queue_blocks_queue_mutations_but_allows_cancel() {
    let home = TempStokerHome::new();
    let _cleanup = ServiceCleanup {
        home: home.path().to_path_buf(),
    };
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let active = store
        .create_job(long_running_job("active", cwd.path().to_path_buf()))
        .unwrap();
    let queued = store
        .create_job(queued_job("queued", cwd.path().to_path_buf()))
        .unwrap();
    store.commit_job(active).unwrap();
    store.commit_job(queued).unwrap();

    start_service(&home);
    wait_for_state(&store, active, JobState::Running);
    let draft = store
        .create_job(queued_job("draft", cwd.path().to_path_buf()))
        .unwrap();

    stoker_with_home(&home)
        .args(["queue", "lock"])
        .assert()
        .success();
    stoker_with_home(&home)
        .args(["status"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Scheduler: running")
                .and(predicate::str::contains("Queue: locked")),
        );

    stoker_with_home(&home)
        .args(["commit", &draft.to_string()])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("queue is locked")
                .and(predicate::str::contains("stoker queue unlock")),
        );
    stoker_with_home(&home)
        .args(["commit", "--all"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("queue is locked")
                .and(predicate::str::contains("stoker queue unlock")),
        );
    stoker_with_home(&home)
        .args(["cancel", &queued.to_string(), "--yes"])
        .assert()
        .success();
    assert_eq!(store.get_job(queued).unwrap().state, JobState::Cancelled);

    stoker_with_home(&home)
        .args(["stop", "--yes"])
        .assert()
        .success();
}

fn run_queue_edit_cancel_race(cancel_selected: bool) {
    let home = TempStokerHome::new();
    let _cleanup = ServiceCleanup {
        home: home.path().to_path_buf(),
    };
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let active = store
        .create_job(long_running_job("active", cwd.path().to_path_buf()))
        .unwrap();
    let first = store
        .create_job(queued_job("first", cwd.path().to_path_buf()))
        .unwrap();
    let second = store
        .create_job(queued_job("second", cwd.path().to_path_buf()))
        .unwrap();
    let selected = store
        .create_job(queued_job("selected", cwd.path().to_path_buf()))
        .unwrap();
    for id in [active, first, second, selected] {
        store.commit_job(id).unwrap();
    }

    start_service(&home);
    wait_for_state(&store, active, JobState::Running);

    let paths = service_paths(&home);
    let runtime = Runtime::new().unwrap();
    let lock_client = ServiceClient::new(paths.clone());
    runtime.block_on(lock_client.lock_queue()).unwrap();
    wait_for_service_status(&runtime, &lock_client, "queue lock", |status| {
        status.queue_locked && status.queued_jobs == 3
    });

    let cancelled = if cancel_selected { selected } else { first };
    let move_client = ServiceClient::new(paths.clone());
    let cancel_client = ServiceClient::new(paths);
    let barrier = Arc::new(Barrier::new(3));
    let move_barrier = Arc::clone(&barrier);
    let move_handle = runtime.handle().spawn(async move {
        move_barrier.wait().await;
        move_client.move_queued(selected, 1).await
    });
    let cancel_barrier = Arc::clone(&barrier);
    let cancel_handle = runtime.handle().spawn(async move {
        cancel_barrier.wait().await;
        cancel_client.cancel(cancelled).await
    });
    runtime.block_on(barrier.wait());
    let (move_result, cancel_result) =
        runtime.block_on(async { (move_handle.await.unwrap(), cancel_handle.await.unwrap()) });

    if let Err(error) = move_result {
        assert!(
            error.to_string().contains("cannot move job")
                || error.to_string().contains("no longer queued"),
            "unexpected move result: {error:#}"
        );
    }
    cancel_result.unwrap();
    wait_for_state(&store, cancelled, JobState::Cancelled);
    let status = wait_for_service_status(&runtime, &lock_client, "locked race result", |status| {
        status.queue_locked && status.active_job == Some(active) && status.queued_jobs == 2
    });
    assert!(status.queue_locked);

    let queued = store
        .list_jobs_with_state(None, Some(JobState::Queued))
        .unwrap();
    let expected_ids = if cancel_selected {
        vec![first, second]
    } else {
        vec![selected, second]
    };
    assert_eq!(
        queued.iter().map(|job| job.id).collect::<Vec<_>>(),
        expected_ids
    );
    assert_eq!(
        queued.iter().map(|job| job.queue_order).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    assert_eq!(store.get_job(cancelled).unwrap().state, JobState::Cancelled);
    assert_eq!(store.get_job(cancelled).unwrap().queue_order, None);
}

#[test]
fn queue_edit_cancel_selected_race_does_not_resurrect_cancelled_job() {
    run_queue_edit_cancel_race(true);
}

#[test]
fn queue_edit_cancel_non_selected_race_does_not_resurrect_cancelled_job() {
    run_queue_edit_cancel_race(false);
}

#[test]
fn queue_edit_selected_move_cancel_then_stale_rollback_does_not_resurrect_job() {
    let home = TempStokerHome::new();
    let _cleanup = ServiceCleanup {
        home: home.path().to_path_buf(),
    };
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let active = store
        .create_job(long_running_job("active", cwd.path().to_path_buf()))
        .unwrap();
    let first = store
        .create_job(queued_job("first", cwd.path().to_path_buf()))
        .unwrap();
    let second = store
        .create_job(queued_job("second", cwd.path().to_path_buf()))
        .unwrap();
    let selected = store
        .create_job(queued_job("selected", cwd.path().to_path_buf()))
        .unwrap();
    for id in [active, first, second, selected] {
        store.commit_job(id).unwrap();
    }

    start_service(&home);
    wait_for_state(&store, active, JobState::Running);

    let paths = service_paths(&home);
    let runtime = Runtime::new().unwrap();
    let editor_client = ServiceClient::new(paths.clone());
    let cancel_client = ServiceClient::new(paths);
    runtime.block_on(editor_client.lock_queue()).unwrap();
    wait_for_service_status(&runtime, &editor_client, "queue lock", |status| {
        status.queue_locked && status.queued_jobs == 3
    });

    let moved = runtime
        .block_on(editor_client.move_queued(selected, 1))
        .unwrap();
    assert_eq!(
        moved.iter().map(|job| job.id).collect::<Vec<_>>(),
        [selected, first, second]
    );

    // A second terminal cancels the job after the editor has persisted its
    // move. The editor then attempts its stale move-mode rollback.
    runtime.block_on(cancel_client.cancel(selected)).unwrap();
    wait_for_state(&store, selected, JobState::Cancelled);
    let rollback = runtime.block_on(editor_client.move_queued(selected, 3));
    let rollback_error = rollback.unwrap_err();
    assert!(
        rollback_error.to_string().contains("cannot move job"),
        "unexpected stale rollback result: {rollback_error:#}"
    );

    let status = wait_for_service_status(
        &runtime,
        &editor_client,
        "stale rollback result",
        |status| {
            status.queue_locked && status.active_job == Some(active) && status.queued_jobs == 2
        },
    );
    assert!(status.queue_locked);
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
fn cancel_requires_confirmation() {
    stoker_with_home(TempStokerHome::new())
        .args(["cancel", "00000000-0000-0000-0000-000000000001"])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Cancel job 00000000-0000-0000-0000-000000000001? [y/N]:")
                .and(predicate::str::contains("Cancel cancelled.")),
        );
}

#[test]
fn serve_command_is_no_longer_available() {
    stoker_with_home(TempStokerHome::new())
        .args(["serve"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'serve'"));
}

#[test]
fn duplicate_service_does_not_replace_running_endpoint() {
    let home = TempStokerHome::new();
    start_service(&home);
    stoker_with_home(&home)
        .args(["start"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already running"));
    stoker_with_home(&home).args(["stop"]).assert().success();
    start_service(&home);
    stoker_with_home(&home).args(["stop"]).assert().success();
}

#[test]
fn commit_all_queues_drafts_in_creation_order() {
    let home = TempStokerHome::new();
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let active = store
        .create_job(long_running_job("active", cwd.path().to_path_buf()))
        .unwrap();
    let first = store
        .create_job(queued_job("first", cwd.path().to_path_buf()))
        .unwrap();
    let second = store
        .create_job(queued_job("second", cwd.path().to_path_buf()))
        .unwrap();
    store.commit_job(active).unwrap();
    let connection = Connection::open(home.path().join("stoker.db")).unwrap();
    for (id, created_at) in [
        (first, "2026-01-01T00:01:00+00:00"),
        (second, "2026-01-01T00:00:00+00:00"),
    ] {
        connection
            .execute(
                "UPDATE jobs SET created_at = ?2 WHERE id = ?1",
                [id.to_string(), created_at.to_owned()],
            )
            .unwrap();
    }
    start_service(&home);
    wait_for_state(&store, active, JobState::Running);

    stoker_with_home(&home)
        .args(["commit", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed 2 DRAFT job(s)."));
    let queued = store
        .list_jobs_with_state(None, Some(JobState::Queued))
        .unwrap();
    assert_eq!(
        queued.iter().map(|job| job.id).collect::<Vec<_>>(),
        [second, first]
    );

    stoker_with_home(&home)
        .args(["stop"])
        .write_stdin("y\n")
        .assert()
        .success();
}

#[test]
fn stop_keeps_active_job_running_without_confirmation() {
    let home = TempStokerHome::new();
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(home.path().join("stoker.db")).unwrap();
    let active = store
        .create_job(long_running_job("active", cwd.path().to_path_buf()))
        .unwrap();
    store.commit_job(active).unwrap();
    start_service(&home);
    wait_for_state(&store, active, JobState::Running);

    stoker_with_home(&home)
        .args(["stop"])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Stop cancelled."));
    stoker_with_home(&home)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scheduler: running"));
    assert_eq!(store.get_job(active).unwrap().state, JobState::Running);

    stoker_with_home(&home)
        .args(["stop"])
        .write_stdin("yes\n")
        .assert()
        .success();
    wait_for_state(&store, active, JobState::Cancelled);
}

#[cfg(unix)]
#[test]
fn start_never_replaces_non_socket_endpoint() {
    let home = TempStokerHome::new();
    let endpoint = home.path().join("stoker.sock");
    std::fs::write(&endpoint, "preserve me").expect("write endpoint sentinel");
    stoker_with_home(&home).args(["start"]).assert().failure();
    assert_eq!(std::fs::read_to_string(endpoint).unwrap(), "preserve me");
}

#[cfg(unix)]
#[test]
fn status_surfaces_online_protocol_errors_instead_of_offline_fallback() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    let home = TempStokerHome::new();
    let endpoint = home.path().join("stoker.sock");
    let listener = UnixListener::bind(&endpoint).expect("bind fake service endpoint");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fake service client");
        let mut header = [0; 4];
        stream
            .read_exact(&mut header)
            .expect("read request frame header");
        let length = u32::from_be_bytes(header) as usize;
        let mut request = vec![0; length];
        stream.read_exact(&mut request).expect("read request frame");
        let response =
            br#"{"version":1,"response":{"Status":{"pid":1,"active_job":null,"queued_jobs":0,"queue_locked":false}}}"#;
        stream
            .write_all(&(response.len() as u32).to_be_bytes())
            .expect("write response frame header");
        stream.write_all(response).expect("write response frame");
    });

    stoker_with_home(&home)
        .args(["status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported IPC protocol version"));
    server.join().expect("fake service thread");
}

#[test]
fn service_restart_marks_stranded_running_job_lost() {
    let home = TempStokerHome::new();
    let db_path = home.path().join("stoker.db");
    let store = Store::open(&db_path).unwrap();
    let job = store
        .create_job(NewJob {
            name: "stranded".into(),
            user: "test".into(),
            cwd: PathBuf::from("/tmp/repository"),
            command: vec!["echo".into(), "never".into()],
        })
        .unwrap();
    Connection::open(&db_path)
        .unwrap()
        .execute(
            "UPDATE jobs SET state = 'RUNNING', pid = 1234 WHERE id = ?1",
            [job.to_string()],
        )
        .unwrap();
    start_service(&home);
    assert_eq!(
        Store::open(&db_path).unwrap().get_job(job).unwrap().state,
        JobState::Lost
    );
    stoker_with_home(&home).args(["stop"]).assert().success();
}

#[test]
fn clean_can_remove_terminal_jobs_while_service_is_running() {
    let home = TempStokerHome::new();
    let db_path = home.path().join("stoker.db");
    let store = Store::open(&db_path).unwrap();
    let job = store
        .create_job(NewJob {
            name: "finished".into(),
            user: "test".into(),
            cwd: PathBuf::from("/tmp/repository"),
            command: vec!["echo".into(), "finished".into()],
        })
        .unwrap();
    store.commit_job(job).unwrap();
    store.claim_next().unwrap().unwrap();
    store.set_running(job, 1).unwrap();
    store.finish(job, Some(0), None).unwrap();
    std::fs::create_dir_all(home.path().join("runs").join(job.to_string())).unwrap();

    start_service(&home);
    stoker_with_home(&home)
        .args(["clean"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1"));
    stoker_with_home(&home).args(["stop"]).assert().success();

    assert!(!home.path().join("runs").join(job.to_string()).exists());
    assert!(matches!(
        Store::open(&db_path).unwrap().get_job(job),
        Err(stoker::StoreError::NotFound { .. })
    ));
}
