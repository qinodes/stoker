mod support;

use predicates::prelude::*;
use rusqlite::Connection;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use stoker::{JobState, NewJob, Store};
use support::{TempStokerHome, stoker_with_home};

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
    stoker_with_home(&home).args(["stop"]).assert().success();
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
fn pause_and_resume_change_only_queued_jobs() {
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
    for id in [active, first, second] {
        store.commit_job(id).unwrap();
    }
    start_service(&home);
    wait_for_state(&store, active, JobState::Running);

    stoker_with_home(&home)
        .args(["pause"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Paused queued jobs."));
    assert_eq!(store.get_job(active).unwrap().state, JobState::Running);
    assert_eq!(store.get_job(first).unwrap().state, JobState::Paused);
    assert_eq!(store.get_job(second).unwrap().state, JobState::Paused);

    stoker_with_home(&home)
        .args(["resume"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Resumed paused jobs."));
    assert_eq!(store.get_job(first).unwrap().state, JobState::Queued);
    assert_eq!(store.get_job(second).unwrap().state, JobState::Queued);
    assert_eq!(store.get_job(first).unwrap().queue_order, Some(1));
    assert_eq!(store.get_job(second).unwrap().queue_order, Some(2));

    stoker_with_home(&home)
        .args(["stop"])
        .write_stdin("y\n")
        .assert()
        .success();
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
            br#"{"version":2,"response":{"Status":{"pid":1,"active_job":null,"queued_jobs":0}}}"#;
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
