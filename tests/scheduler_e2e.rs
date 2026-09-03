mod support;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use predicates::prelude::*;
use stoker::{JobState, Store};
use uuid::Uuid;

use support::{TestCommand, TestRepo, stoker_with_home, stoker_with_home_and_dir};

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

pub fn add_script(repo: &TestRepo, script: &str, name: &str) -> Uuid {
    add_script_from(repo, repo.path(), script, name)
}

fn add_script_from(
    repo: &TestRepo,
    current_dir: &std::path::Path,
    script: &str,
    name: &str,
) -> Uuid {
    let mut args = vec!["add", "--user", "test", "--name", name, "--cmd"];
    let shell = TestCommand::shell(script);
    args.extend(shell.iter().map(String::as_str));
    let output = stoker_with_home_and_dir(&home_for(repo), current_dir)
        .args(args)
        .output()
        .unwrap();
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

fn order_job(repo: &TestRepo, value: &str, order_path: &std::path::Path) -> String {
    #[cfg(unix)]
    {
        let script_name = format!("script-{value}.sh");
        repo.commit_script_named(
            &script_name,
            &format!(
                "printf {value}; printf generated > generated; printf '{value}\\n' >> '{}'",
                order_path.display()
            ),
        );
        format!("sh {script_name}")
    }
    #[cfg(windows)]
    {
        let script_name = format!("script-{value}.cmd");
        repo.commit_script_named(
            &script_name,
            &format!(
                "@echo off\r\necho {value}\r\necho {value}>>\"{}\"\r\necho generated>generated",
                order_path.display()
            ),
        );
        script_name
    }
}

fn output_command(value: &str) -> String {
    #[cfg(unix)]
    {
        format!("printf {value}")
    }
    #[cfg(windows)]
    {
        format!("echo {value}")
    }
}

fn failed_command() -> &'static str {
    #[cfg(unix)]
    {
        "printf fail >&2; exit 7"
    }
    #[cfg(windows)]
    {
        "echo fail 1>&2 & exit /B 7"
    }
}

fn long_running_command() -> &'static str {
    #[cfg(unix)]
    {
        "sleep 30"
    }
    #[cfg(windows)]
    {
        "ping 127.0.0.1 -n 31 > NUL"
    }
}

fn follow_command() -> &'static str {
    #[cfg(unix)]
    {
        "printf first; sleep 1; printf second"
    }
    #[cfg(windows)]
    {
        "echo first & ping 127.0.0.1 -n 2 > NUL & echo second"
    }
}

pub fn start_service_and_commit(ids: &[Uuid]) {
    let home = homes().lock().unwrap().get(&ids[0]).cloned().unwrap();
    let status = Command::new(assert_cmd::cargo::cargo_bin("stoker"))
        .arg("start")
        .env("STOKER_HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "failed to start scheduler service: {status}"
    );
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
fn service_runs_jobs_in_recorded_cwd_in_queue_order() {
    let repo = TestRepo::new();
    let order_dir = tempfile::tempdir().unwrap();
    let order_path = order_dir.path().join("order.log");
    let first = add_script(&repo, &order_job(&repo, "first", &order_path), "first");
    let second = add_script(&repo, &order_job(&repo, "second", &order_path), "second");
    start_service_and_commit(&[first, second]);
    assert_terminal(first, JobState::Succeeded);
    assert_terminal(second, JobState::Succeeded);
    assert_log_contains(first, "first");
    assert_log_contains(second, "second");
    assert_eq!(
        std::fs::read_to_string(&order_path).unwrap(),
        if cfg!(windows) {
            "first\r\nsecond\r\n"
        } else {
            "first\nsecond\n"
        }
    );
    assert!(
        repo.join("generated").exists(),
        "job should run in the recorded source cwd"
    );
    stoker_stop(first).assert().success();
}

#[test]
fn service_runs_job_in_the_subdirectory_recorded_at_submission() {
    let repo = TestRepo::new();
    let nested = repo.join("experiments/llama");
    std::fs::create_dir_all(&nested).unwrap();
    let job = add_script_from(
        &repo,
        &nested,
        if cfg!(windows) {
            "echo nested>nested-marker"
        } else {
            "printf nested > nested-marker"
        },
        "nested",
    );

    start_service_and_commit(&[job]);
    assert_terminal(job, JobState::Succeeded);
    assert!(nested.join("nested-marker").exists());

    let home = homes().lock().unwrap().get(&job).cloned().unwrap();
    assert!(
        !home
            .join("runs")
            .join(job.to_string())
            .join("repo")
            .exists()
    );
    stoker_stop(job).assert().success();
}

#[test]
fn failed_job_preserves_logs_and_does_not_block_next_job() {
    let repo = TestRepo::new();
    let failed = add_script(&repo, failed_command(), "failed");
    let succeeding = add_script(&repo, &output_command("ok"), "succeeding");
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
    let job = add_script(&repo, follow_command(), "follow");
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
    let running = add_script(&repo, long_running_command(), "running");
    let next = add_script(&repo, &output_command("next"), "next");
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
    assert!(
        !home
            .join("runs")
            .join(running.to_string())
            .join("repo")
            .exists(),
        "direct execution must not create a job worktree under runs"
    );
    assert_terminal(next, JobState::Succeeded);
    stoker_stop(running).assert().success();
}

#[test]
fn stop_cancels_active_job_and_removes_service_endpoint() {
    let repo = TestRepo::new();
    let job = add_script(&repo, long_running_command(), "running");
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
    let running = add_script(&repo, long_running_command(), "running");
    let queued = add_script(&repo, &output_command("queued"), "queued");
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
