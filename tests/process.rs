use std::ffi::OsString;
use std::path::PathBuf;

use stoker::process::{DefaultProcessController, ProcessController, ProcessSpec};
use tempfile::TempDir;

struct TestProcess {
    _directory: TempDir,
    stdout: PathBuf,
    stderr: PathBuf,
}

fn controller() -> DefaultProcessController {
    DefaultProcessController
}

fn process_paths() -> TestProcess {
    let directory = tempfile::tempdir().unwrap();
    let stdout = directory.path().join("stdout.log");
    let stderr = directory.path().join("stderr.log");
    TestProcess {
        _directory: directory,
        stdout,
        stderr,
    }
}

fn echo_spec(paths: &TestProcess) -> ProcessSpec {
    #[cfg(unix)]
    let (program, args) = (
        OsString::from("sh"),
        vec![
            OsString::from("-c"),
            OsString::from("printf 'out\\n'; printf 'err\\n' >&2"),
        ],
    );
    #[cfg(windows)]
    let (program, args) = (
        OsString::from("cmd"),
        vec![OsString::from("/C"), OsString::from("echo out&echo err>&2")],
    );
    ProcessSpec {
        program,
        args,
        cwd: std::env::current_dir().unwrap(),
        stdout_log: paths.stdout.clone(),
        stderr_log: paths.stderr.clone(),
    }
}

#[tokio::test]
async fn spawned_process_writes_separate_stdout_and_stderr() {
    let paths = process_paths();
    let process = controller().spawn(echo_spec(&paths)).await.unwrap();
    assert_eq!(process.wait().await.unwrap().code(), Some(0));
    #[cfg(unix)]
    let (expected_stdout, expected_stderr) = ("out\n", "err\n");
    #[cfg(windows)]
    let (expected_stdout, expected_stderr) = ("out\r\n", "err\r\n");
    assert_eq!(
        tokio::fs::read_to_string(&paths.stdout).await.unwrap(),
        expected_stdout
    );
    assert_eq!(
        tokio::fs::read_to_string(&paths.stderr).await.unwrap(),
        expected_stderr
    );
}

#[tokio::test]
async fn terminate_tree_stops_a_long_running_process() {
    let paths = process_paths();
    #[cfg(unix)]
    let (program, args) = (
        OsString::from("sh"),
        vec![OsString::from("-c"), OsString::from("sleep 30")],
    );
    #[cfg(windows)]
    let (program, args) = (
        OsString::from("cmd"),
        vec![
            OsString::from("/C"),
            OsString::from("ping 127.0.0.1 -n 31 > NUL"),
        ],
    );
    let spec = ProcessSpec {
        program,
        args,
        cwd: std::env::current_dir().unwrap(),
        stdout_log: paths.stdout,
        stderr_log: paths.stderr,
    };
    let mut process = controller().spawn(spec).await.unwrap();
    process.terminate_tree().await.unwrap();
    assert_ne!(process.wait().await.unwrap().code(), Some(0));
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_tree_stops_descendants_too() {
    let paths = process_paths();
    let ready = paths._directory.path().join("descendant-ready.txt");
    let marker = paths._directory.path().join("descendant-late.txt");
    let script = format!(
        "(printf ready > {}; sleep 1; printf descendant > {}) & wait",
        ready.to_string_lossy(),
        marker.to_string_lossy(),
    );
    let spec = ProcessSpec {
        program: OsString::from("sh"),
        args: vec![OsString::from("-c"), OsString::from(script)],
        cwd: std::env::current_dir().unwrap(),
        stdout_log: paths.stdout,
        stderr_log: paths.stderr,
    };
    let mut process = controller().spawn(spec).await.unwrap();
    for _ in 0..100 {
        if ready.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(ready.exists(), "descendant did not reach its ready state");
    process.terminate_tree().await.unwrap();
    assert_ne!(process.wait().await.unwrap().code(), Some(0));
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert!(
        !marker.exists(),
        "descendant survived process-group termination"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_tree_escalates_when_sigterm_is_ignored() {
    let paths = process_paths();
    let spec = ProcessSpec {
        program: OsString::from("sh"),
        args: vec![
            OsString::from("-c"),
            OsString::from("trap '' TERM; sleep 30"),
        ],
        cwd: std::env::current_dir().unwrap(),
        stdout_log: paths.stdout,
        stderr_log: paths.stderr,
    };
    let mut process = controller().spawn(spec).await.unwrap();
    let started = tokio::time::Instant::now();
    process.terminate_tree().await.unwrap();
    let status = process.wait().await.unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "SIGTERM escalation exceeded bounded cancellation grace period"
    );
    assert_ne!(status.code(), Some(0));
}
