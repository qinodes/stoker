mod support;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use predicates::prelude::*;
use stoker::{JobState, Store};
use uuid::Uuid;

use support::{TestCommand, TestRepo, stoker_in, stoker_with_home};

static JOB_HOMES: OnceLock<Mutex<HashMap<Uuid, PathBuf>>> = OnceLock::new();

fn homes() -> &'static Mutex<HashMap<Uuid, PathBuf>> {
    JOB_HOMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn home_for(repo: &TestRepo) -> PathBuf {
    let root = repo.path();
    root.parent().unwrap().join(format!(
        ".{}-stoker-home",
        root.file_name().unwrap().to_string_lossy()
    ))
}

fn git_output(repo: &TestRepo, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub fn submit_script(repo: &TestRepo, script: &str, name: &str) -> Uuid {
    let mut args = vec!["submit", "--user", "test", "--name", name, "--"];
    let shell = TestCommand::shell(script);
    args.extend(shell.iter().map(String::as_str));
    let output = stoker_in(repo.path()).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .parse()
        .unwrap();
    homes().lock().unwrap().insert(id, home_for(repo));
    id
}

pub fn start_service_and_commit(ids: &[Uuid]) {
    let home = homes().lock().unwrap().get(&ids[0]).cloned().unwrap();
    stoker_with_home(&home).args(["serve"]).assert().success();
    for id in ids {
        stoker_with_home(&home)
            .args(["commit", &id.to_string()])
            .assert()
            .success();
    }
}

pub fn wait_for_state(id: Uuid, expected: JobState) {
    let home = homes().lock().unwrap().get(&id).cloned().unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if store.get_job(id).unwrap().state == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "job {id} did not reach {expected}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn assert_terminal(id: Uuid, expected: JobState) {
    wait_for_state(id, expected);
}

pub fn assert_log_contains(id: Uuid, text: &str) {
    let home = homes().lock().unwrap().get(&id).cloned().unwrap();
    let run = home.join("runs").join(id.to_string());
    let stdout = std::fs::read_to_string(run.join("stdout.log")).unwrap_or_default();
    let stderr = std::fs::read_to_string(run.join("stderr.log")).unwrap_or_default();
    assert!(
        stdout.contains(text) || stderr.contains(text),
        "missing {text:?}"
    );
}

pub fn stoker_logs_follow(id: Uuid) -> assert_cmd::Command {
    let home = homes().lock().unwrap().get(&id).cloned().unwrap();
    let mut command = stoker_with_home(&home);
    command.args(["logs", "-f", &id.to_string()]);
    command
}

pub fn stoker_cancel(id: Uuid) -> assert_cmd::Command {
    let home = homes().lock().unwrap().get(&id).cloned().unwrap();
    let mut command = stoker_with_home(&home);
    command.args(["cancel", &id.to_string()]);
    command
}

pub fn stoker_stop(id: Uuid) -> assert_cmd::Command {
    let home = homes().lock().unwrap().get(&id).cloned().unwrap();
    let mut command = stoker_with_home(&home);
    command.args(["stop"]);
    command
}

pub fn stoker_status(id: Uuid) -> assert_cmd::Command {
    let home = homes().lock().unwrap().get(&id).cloned().unwrap();
    let mut command = stoker_with_home(&home);
    command.args(["status"]);
    command
}

#[test]
fn service_runs_committed_jobs_in_order_from_captured_commits() {
    let repo = TestRepo::new();
    let order_dir = tempfile::tempdir().unwrap();
    let order_path = order_dir.path().join("order.log");
    #[cfg(unix)]
    let command = |value: &str| {
        format!(
            "sh script.sh; printf '{value}\\n' >> '{}'",
            order_path.display()
        )
    };
    #[cfg(windows)]
    let command = |value: &str| format!("script.sh & echo {value}>>{}", order_path.display());
    repo.commit_script("printf first; printf generated > generated");
    let first = submit_script(&repo, &command("first"), "first");
    let first_commit = git_output(&repo, &["rev-parse", "HEAD"]);
    repo.commit_script("printf second; printf generated > generated");
    let second = submit_script(&repo, &command("second"), "second");
    let second_commit = git_output(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(first_commit, second_commit);
    let source_head_before = git_output(&repo, &["rev-parse", "HEAD"]);
    let source_status_before = git_output(&repo, &["status", "--porcelain"]);
    start_service_and_commit(&[first, second]);
    assert_terminal(first, JobState::Succeeded);
    assert_terminal(second, JobState::Succeeded);
    assert_log_contains(first, "first");
    assert_log_contains(second, "second");
    let home = homes().lock().unwrap().get(&first).cloned().unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    assert_eq!(store.get_job(first).unwrap().git_commit, first_commit);
    assert_eq!(store.get_job(second).unwrap().git_commit, second_commit);
    assert_eq!(
        std::fs::read_to_string(&order_path).unwrap(),
        "first\nsecond\n"
    );
    assert_eq!(
        git_output(&repo, &["rev-parse", "HEAD"]),
        source_head_before
    );
    assert_eq!(
        git_output(&repo, &["status", "--porcelain"]),
        source_status_before
    );
    assert!(
        !repo.join("generated").exists(),
        "job modified source checkout"
    );
    stoker_stop(first).assert().success();
}

#[test]
fn failed_job_preserves_logs_and_does_not_block_next_job() {
    let repo = TestRepo::new();
    let failed = submit_script(&repo, "printf fail >&2; exit 7", "failed");
    let succeeding = submit_script(&repo, "printf ok", "succeeding");
    start_service_and_commit(&[failed, succeeding]);
    assert_terminal(failed, JobState::Failed);
    assert_log_contains(failed, "fail");
    assert_terminal(succeeding, JobState::Succeeded);
    assert_log_contains(succeeding, "ok");
    stoker_stop(failed).assert().success();
}

#[test]
fn logs_follow_receives_output_before_job_finishes() {
    let repo = TestRepo::new();
    let job = submit_script(&repo, "printf first; sleep 1; printf second", "follow");
    start_service_and_commit(&[job]);
    stoker_logs_follow(job)
        .assert()
        .success()
        .stdout(predicate::str::contains("first").and(predicate::str::contains("second")));
    stoker_stop(job).assert().success();
}

#[test]
fn cancel_running_job_terminates_tree_then_starts_next_job() {
    let repo = TestRepo::new();
    let running = submit_script(&repo, "sleep 30", "running");
    let next = submit_script(&repo, "printf next", "next");
    start_service_and_commit(&[running, next]);
    wait_for_state(running, JobState::Running);
    stoker_cancel(running).assert().success();
    assert_terminal(running, JobState::Cancelled);
    let home = homes().lock().unwrap().get(&running).cloned().unwrap();
    let cancelled = Store::open(home.join("stoker.db"))
        .unwrap()
        .get_job(running)
        .unwrap();
    assert!(cancelled.pid.is_none());
    assert!(cancelled.execution_dir.is_none());
    assert!(
        !home
            .join("runs")
            .join(running.to_string())
            .join("repo")
            .exists()
    );
    assert_terminal(next, JobState::Succeeded);
    stoker_stop(running).assert().success();
}

#[test]
fn stop_cancels_active_job_and_removes_service_endpoint() {
    let repo = TestRepo::new();
    let job = submit_script(&repo, "sleep 30", "running");
    start_service_and_commit(&[job]);
    wait_for_state(job, JobState::Running);
    stoker_stop(job).assert().success();
    assert_terminal(job, JobState::Cancelled);
    stoker_status(job)
        .assert()
        .stdout(predicate::str::contains("Scheduler: stopped"));
}

#[test]
fn stop_releases_queued_log_followers() {
    let repo = TestRepo::new();
    let running = submit_script(&repo, "sleep 30", "running");
    let queued = submit_script(&repo, "printf queued", "queued");
    start_service_and_commit(&[running, queued]);
    wait_for_state(running, JobState::Running);
    let home = homes().lock().unwrap().get(&queued).cloned().unwrap();
    let executable = assert_cmd::cargo::cargo_bin("stoker");
    let follower = Command::new(executable)
        .args(["logs", "-f", &queued.to_string()])
        .env("STOKER_HOME", home)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    stoker_stop(running).assert().success();
    let output = follower.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("shutting down"),
        "queued follower did not receive a shutdown response: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_terminal(running, JobState::Cancelled);
    assert_terminal(queued, JobState::Queued);
}
