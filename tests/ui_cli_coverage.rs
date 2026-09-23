use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use stoker::{NewJob, StokerPaths, Store};

fn ui(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stoker"))
        .env("STOKER_HOME", home)
        .arg("ui")
        .args(args)
        .output()
        .unwrap()
}

struct StopUi(PathBuf);

impl Drop for StopUi {
    fn drop(&mut self) {
        let _ = ui(&self.0, &["stop"]);
    }
}

struct StopService(PathBuf);

impl Drop for StopService {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_stoker"))
            .env("STOKER_HOME", &self.0)
            .args(["stop", "--yes"])
            .output();
    }
}

fn post(port: u16, path: &str) -> String {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn public_ui_start_status_repeat_start_and_stop_share_the_live_listener() {
    let directory = tempfile::tempdir().unwrap();
    let guard = StopUi(directory.path().to_path_buf());
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    drop(listener);

    let start = ui(directory.path(), &["start", "--port", &port]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(String::from_utf8_lossy(&start.stdout).contains("Stoker UI started"));
    let metadata = std::fs::read_to_string(directory.path().join("ui.json")).unwrap();
    assert!(metadata.contains(&port));

    let status = ui(directory.path(), &["status"]);
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("Stoker UI: running"));
    let repeat = ui(directory.path(), &["start", "--port", &port]);
    assert!(repeat.status.success());
    assert!(String::from_utf8_lossy(&repeat.stdout).contains("already running"));

    let stop = ui(directory.path(), &["stop"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert!(!directory.path().join("ui.json").exists());
    let stopped = ui(directory.path(), &["status"]);
    assert!(stopped.status.success());
    assert!(String::from_utf8_lossy(&stopped.stdout).contains("Stoker UI: stopped"));
    drop(guard);
}

#[test]
fn ui_job_commit_and_cancel_use_the_live_scheduler_gateway() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let service = StopService(home.to_path_buf());
    let ui_guard = StopUi(home.to_path_buf());
    let started = Command::new(env!("CARGO_BIN_EXE_stoker"))
        .env("STOKER_HOME", home)
        .arg("start")
        .output()
        .unwrap();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let paths = StokerPaths {
        root: home.to_path_buf(),
        database: home.join("stoker.db"),
        runs: home.join("runs"),
        lock: home.join("stoker.lock"),
        endpoint: home.join("stoker.sock"),
    };
    let store = Store::open(&paths.database).unwrap();
    let committed = store
        .create_job(NewJob {
            name: "commit through UI".into(),
            user: "tester".into(),
            description: None,
            cwd: home.to_path_buf(),
            command: vec!["echo".into(), "one".into()],
        })
        .unwrap();
    let cancelled = store
        .create_job(NewJob {
            name: "cancel through UI".into(),
            user: "tester".into(),
            description: None,
            cwd: home.to_path_buf(),
            command: vec!["echo".into(), "two".into()],
        })
        .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let start = ui(home, &["start", "--port", &port.to_string()]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );

    let response = post(port, &format!("/api/v1/jobs/{cancelled}/cancel"));
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(
        store.get_job(cancelled).unwrap().state,
        stoker::JobState::Cancelled
    );
    let response = post(port, &format!("/api/v1/jobs/{committed}/commit"));
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(&committed.to_string()));
    assert_ne!(
        store.get_job(committed).unwrap().state,
        stoker::JobState::Draft
    );

    drop(ui_guard);
    drop(service);
}
