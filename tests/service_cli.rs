mod support;

use predicates::prelude::*;
use rusqlite::Connection;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use stoker::{JobState, NewJob, Store};
use support::{TempStokerHome, stoker_with_home};

fn start_service(home: &TempStokerHome) {
    let status = Command::new(assert_cmd::cargo::cargo_bin("stoker"))
        .arg("serve")
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

#[test]
fn serve_detaches_and_status_reports_running_service() {
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
fn stop_requires_running_service() {
    stoker_with_home(TempStokerHome::new())
        .args(["stop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Start it with 'stoker serve'"));
}

#[test]
fn duplicate_service_does_not_replace_running_endpoint() {
    let home = TempStokerHome::new();
    start_service(&home);
    stoker_with_home(&home)
        .args(["serve"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already running"));
    stoker_with_home(&home).args(["stop"]).assert().success();
    start_service(&home);
    stoker_with_home(&home).args(["stop"]).assert().success();
}

#[cfg(unix)]
#[test]
fn serve_never_replaces_non_socket_endpoint() {
    let home = TempStokerHome::new();
    let endpoint = home.path().join("stoker.sock");
    std::fs::write(&endpoint, "preserve me").expect("write endpoint sentinel");
    stoker_with_home(&home).args(["serve"]).assert().failure();
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
            repository: PathBuf::from("/tmp/repository"),
            git_commit: "0123456789abcdef".into(),
            cwd: PathBuf::from("."),
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
