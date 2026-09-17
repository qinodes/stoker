use assert_cmd::Command;
use chrono::{Duration, NaiveTime, Utc};
use predicates::prelude::*;
use stoker::domain::flow::{DependencyMode, ExecutionMode, ScheduleSpec};
use stoker::store::{FlowAttemptResult, FlowTaskInput};
use stoker::{NewJob, Store};

fn cli(home: &std::path::Path, cwd: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("stoker").unwrap();
    command
        .env("STOKER_HOME", home)
        .env("NO_COLOR", "1")
        .current_dir(cwd);
    command
}

fn add_task(store: &Store, root: &std::path::Path, flow_id: &str, task_id: &str) {
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow_id.into(),
            task_id: task_id.into(),
            name: task_id.into(),
            cwd: root.to_string_lossy().into_owned(),
            command: "echo cli".into(),
            retry: 0,
            dependencies: vec![],
            depend_mode: DependencyMode::All,
        })
        .unwrap();
}

fn runnable_flow(store: &Store, root: &std::path::Path, id: &str) {
    store
        .create_flow(
            id.into(),
            id.into(),
            "tester".into(),
            ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(2),
            },
        )
        .unwrap();
    add_task(store, root, id, "root");
    store.commit_flow(id).unwrap();
    if store.current_mode().unwrap() != ExecutionMode::Scheduled {
        store.lock_queue().unwrap();
        store.set_mode(ExecutionMode::Scheduled).unwrap();
        store.unlock_queue().unwrap();
    }
}

#[test]
fn extended_help_is_successful_and_top_level_mentions_the_extended_surface() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    cli(&home, directory.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Extended commands: flow, mode, run",
        ));
    cli(&home, directory.path())
        .args(["flow", "logs", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--attempt"))
        .stdout(predicate::str::contains("--follow"));
}

#[test]
fn flow_task_submission_keeps_a_shell_compatible_working_directory() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    cli(&home, directory.path())
        .args(["mode", "set", "scheduled"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["queue", "unlock"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "create",
            "cwd-flow",
            "--user",
            "tester",
            "--name",
            "Build: nightly",
            "--at",
            "2099-01-01T00:00:00Z",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow", "task", "add", "cwd-flow", "cwd", "--name", "cwd", "--cmd", "cd",
        ])
        .assert()
        .success();

    let cwd = Store::open(home.join("stoker.db"))
        .unwrap()
        .get_flow("cwd-flow")
        .unwrap()
        .tasks[0]
        .cwd
        .clone();
    assert!(!cwd.starts_with("//?/"));
    assert_eq!(cwd, directory.path().to_string_lossy().replace('\\', "/"));
}

#[test]
fn jobs_accepts_lowercase_state_and_policy_exposes_max_concurrency() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    let job_id = store
        .create_job(NewJob {
            name: "scheduled".into(),
            user: "tester".into(),
            description: None,
            cwd: directory.path().into(),
            command: vec!["echo".into(), "ok".into()],
        })
        .unwrap();
    store
        .configure_standalone(
            job_id,
            ExecutionMode::Scheduled,
            Some(ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(1),
            }),
            0,
        )
        .unwrap();
    store.lock_queue().unwrap();

    cli(&home, directory.path())
        .args(["jobs", "--mode", "scheduled", "--state", "draft"])
        .assert()
        .success()
        .stdout(predicate::str::contains(job_id.to_string()));
    cli(&home, directory.path())
        .args(["policy", "get", "max-concurrency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("max-concurrency: 2"));
    cli(&home, directory.path())
        .args(["policy", "set", "max-concurrency", "3"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["policy", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"max_concurrency\": 3"));
}

#[test]
fn flow_queries_validate_definition_and_task_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    runnable_flow(&store, directory.path(), "flow-a");
    runnable_flow(&store, directory.path(), "flow-b");
    let run_a = store
        .create_flow_run("flow-a", "MANUAL", false, None)
        .unwrap();
    let run_b = store
        .create_flow_run("flow-b", "MANUAL", false, None)
        .unwrap();

    cli(&home, directory.path())
        .args(["flow", "show", "flow-a", "--run", &run_b.run_id.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not belong"));
    cli(&home, directory.path())
        .args([
            "flow",
            "show",
            "flow-a",
            "--run",
            &run_a.run_id.to_string(),
            "--task",
            "missing",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("task does not exist"));
}

#[test]
fn timezone_only_schedule_edit_and_scheduled_queue_edit_contracts_are_reachable() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    store
        .create_flow(
            "daily".into(),
            "daily".into(),
            "tester".into(),
            ScheduleSpec::Daily {
                time: NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
                timezone: "UTC".into(),
            },
        )
        .unwrap();
    add_task(&store, directory.path(), "daily", "root");
    store.commit_flow("daily").unwrap();
    store.freeze_flow("daily").unwrap();

    cli(&home, directory.path())
        .args([
            "flow",
            "schedule",
            "set",
            "daily",
            "--schedule-timezone",
            "Asia/Tokyo",
        ])
        .assert()
        .success();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    cli(&home, directory.path())
        .args(["queue", "edit"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "queue edit is unavailable in scheduled mode",
        ));
}

#[test]
fn flow_logs_decode_non_utf8_bytes_lossily() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    runnable_flow(&store, directory.path(), "lossy-logs");
    let run = store
        .create_flow_run("lossy-logs", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    let log_dir = home
        .join("runs")
        .join("flows")
        .join(run.run_id.to_string())
        .join("root")
        .join("attempt-1");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("stdout.log"), [b'a', 0x80, b'z']).unwrap();

    cli(&home, directory.path())
        .args([
            "flow",
            "logs",
            "lossy-logs",
            "--run",
            &run.run_id.to_string(),
            "--task",
            "root",
            "--attempt",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("a�z"));
}

#[test]
fn public_flows_require_scheduled_mode_and_list_has_no_mode_option() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");

    cli(&home, directory.path())
        .args([
            "flow",
            "create",
            "serial-flow",
            "--user",
            "tester",
            "--name",
            "serial-flow",
            "--at",
            "2099-01-01T00:00:00Z",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "flow create is only available in scheduled mode",
        ));
    cli(&home, directory.path())
        .args(["mode", "set", "scheduled"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["queue", "unlock"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "create",
            "missing-schedule",
            "--user",
            "tester",
            "--name",
            "missing-schedule",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required arguments"));
    cli(&home, directory.path())
        .args(["flow", "list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--user"))
        .stdout(predicate::str::contains("--mode").not());
    cli(&home, directory.path())
        .args(["flow", "list", "--mode", "scheduled"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--mode'"));
}

#[test]
fn run_and_occurrence_lists_have_aligned_headers() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    runnable_flow(&store, directory.path(), "headers");

    let empty_runs = cli(&home, directory.path())
        .args(["flow", "runs", "headers"])
        .output()
        .unwrap();
    assert!(empty_runs.status.success());
    let empty_runs = String::from_utf8(empty_runs.stdout).unwrap();
    assert_eq!(empty_runs.lines().count(), 2);
    assert!(empty_runs.starts_with("RUN_ID"));

    let run = store
        .create_flow_run("headers", "MANUAL", false, None)
        .unwrap();
    let occurrence = store.list_occurrences("headers").unwrap().remove(0);

    let runs = cli(&home, directory.path())
        .args(["flow", "runs", "headers"])
        .output()
        .unwrap();
    assert!(runs.status.success());
    assert_aligned_columns(
        &String::from_utf8(runs.stdout).unwrap(),
        &["RUN_ID", "FLOW_ID", "STATE", "SOURCE"],
        &[
            run.run_id.to_string(),
            "headers".into(),
            "Starting".into(),
            "MANUAL".into(),
        ],
    );

    let occurrences = cli(&home, directory.path())
        .args(["flow", "occurrences", "headers"])
        .output()
        .unwrap();
    assert!(occurrences.status.success());
    assert_aligned_columns(
        &String::from_utf8(occurrences.stdout).unwrap(),
        &["OCCURRENCE_ID", "FLOW_ID", "STATE", "DUE_AT_UTC"],
        &[
            occurrence.occurrence_id.to_string(),
            "headers".into(),
            "Pending".into(),
            occurrence.due_at.to_rfc3339(),
        ],
    );
}

#[test]
fn refreshed_flow_commands_use_positional_ids_and_named_dependency_flags() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");

    cli(&home, directory.path())
        .args(["mode", "set", "scheduled"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["queue", "unlock"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "create",
            "nightly",
            "--user",
            "alice",
            "--name",
            "Nightly build",
            "--daily",
            "23:30",
            "--schedule-timezone",
            "Asia/Tokyo",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "task",
            "add",
            "nightly",
            "prepare",
            "--name",
            "Prepare",
            "--cmd",
            "echo prepare",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "task",
            "add",
            "nightly",
            "train",
            "--name",
            "Train",
            "--cmd",
            "echo train",
            "--after",
            "prepare",
            "--retries",
            "2",
            "--match",
            "all",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args([
            "flow",
            "task",
            "add",
            "nightly",
            "notify",
            "--name",
            "Notify",
            "--cmd",
            "echo notify",
            "--after-failure",
            "train",
            "--match",
            "any",
        ])
        .assert()
        .success();

    let store = Store::open(home.join("stoker.db")).unwrap();
    let flow = store.get_flow("nightly").unwrap();
    let train = flow
        .tasks
        .iter()
        .find(|task| task.task_id == "train")
        .unwrap();
    assert_eq!(train.retry, 2);
    assert_eq!(train.dependencies.len(), 1);
    assert_eq!(train.dependencies[0].upstream_task_id, "prepare");
    assert_eq!(train.dependencies[0].status.to_string(), "succeeded");
    let notify = flow
        .tasks
        .iter()
        .find(|task| task.task_id == "notify")
        .unwrap();
    assert_eq!(notify.dependencies[0].upstream_task_id, "train");
    assert_eq!(notify.dependencies[0].status.to_string(), "failed");

    cli(&home, directory.path())
        .args(["flow", "commit", "nightly"])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["flow", "run", "nightly", "--replace-next"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created flow run"));
}

#[test]
fn edit_namespace_preserves_freeze_draft_and_revision_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    runnable_flow(&store, directory.path(), "editable");

    cli(&home, directory.path())
        .args(["flow", "edit", "begin", "editable"])
        .assert()
        .success()
        .stdout(predicate::str::contains("frozen for editing"))
        .stdout(predicate::str::contains("Running processes continue"));
    cli(&home, directory.path())
        .args(["flow", "edit", "discard", "editable"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--revision"));
    cli(&home, directory.path())
        .args([
            "flow",
            "task",
            "update",
            "editable",
            "root",
            "--retries",
            "2",
            "--revision",
            "0",
        ])
        .assert()
        .success();
    cli(&home, directory.path())
        .args(["flow", "edit", "apply", "editable", "--revision", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("revision conflict"));
    cli(&home, directory.path())
        .args(["flow", "edit", "apply", "editable", "--revision", "1"])
        .assert()
        .success();

    let flow = store.get_flow("editable").unwrap();
    assert!(!flow.frozen);
    assert_eq!(flow.tasks[0].retry, 2);
}

#[test]
fn flow_selectors_filter_show_logs_and_cancel_without_task_subcommands() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    runnable_flow(&store, directory.path(), "selectors");
    let run = store
        .create_flow_run("selectors", "MANUAL", false, None)
        .unwrap();

    cli(&home, directory.path())
        .args([
            "flow",
            "show",
            "selectors",
            "--run",
            &run.run_id.to_string(),
            "--task",
            "root",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("task_id=root"));
    cli(&home, directory.path())
        .args(["flow", "show", "selectors", "--task", "root"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--run"));
    cli(&home, directory.path())
        .args([
            "flow",
            "logs",
            "selectors",
            "--run",
            &run.run_id.to_string(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--task"));
    cli(&home, directory.path())
        .args([
            "flow",
            "logs",
            "selectors",
            "--run",
            &run.run_id.to_string(),
            "--task",
            "root",
            "--attempt",
            "0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("0 is not in 1.."));
    cli(&home, directory.path())
        .args([
            "flow",
            "cancel",
            "selectors",
            "--run",
            &run.run_id.to_string(),
            "--task",
            "root",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cancelled task root"));
}

#[test]
fn obsolete_flow_aliases_and_option_names_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let rejected = [
        vec!["flow", "add", "old"],
        vec!["add", "--flow-id", "old", "--task-id", "task"],
        vec!["flow", "task", "set", "old", "task"],
        vec!["flow", "task", "show", "old", "task"],
        vec!["flow", "task", "logs", "old", "task"],
        vec!["flow", "task", "cancel", "old", "task"],
        vec!["flow", "freeze", "old"],
        vec!["flow", "unfreeze", "old"],
        vec!["flow", "draft", "discard", "old"],
        vec!["flow", "run", "old", "--skip-next"],
        vec![
            "flow",
            "task",
            "add",
            "old",
            "task",
            "--name",
            "task",
            "--cmd",
            "echo task",
            "--retry",
            "1",
        ],
    ];
    for args in rejected {
        cli(&home, directory.path()).args(args).assert().failure();
    }

    cli(&home, directory.path())
        .args(["freeze", "a-flow-id"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn dependency_flags_reject_duplicates_and_conflicting_outcomes() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    store
        .create_flow(
            "dependencies".into(),
            "dependencies".into(),
            "tester".into(),
            ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(2),
            },
        )
        .unwrap();

    for dependency_args in [
        vec!["--after", "root", "--after", "root"],
        vec!["--after", "root", "--after-failure", "root"],
    ] {
        let mut args = vec![
            "flow",
            "task",
            "add",
            "dependencies",
            "child",
            "--name",
            "child",
            "--cmd",
            "echo child",
        ];
        args.extend(dependency_args);
        cli(&home, directory.path())
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("duplicate dependency"));
    }
}

#[test]
fn flow_list_is_a_flow_only_summary_and_empty_lists_are_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let empty_home = directory.path().join("empty-home");
    cli(&empty_home, directory.path())
        .args(["flow", "list"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("FLOW_ID"))
        .stdout(predicate::str::contains("No flows found."));

    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    runnable_flow(&store, directory.path(), "summary");
    let output = cli(&home, directory.path())
        .args(["flow", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    assert_aligned_columns(
        &output,
        &["FLOW_ID", "USER", "SCHEDULE", "STATUS"],
        &[
            "summary".into(),
            "tester".into(),
            "once".into(),
            "LIVE".into(),
        ],
    );
    assert!(output.lines().next().unwrap().contains("ACTIVE"));
    assert!(output.lines().next().unwrap().contains("NEXT"));
    assert!(!output.contains("task_id="));
    assert_eq!(output.lines().count(), 3);
}

fn assert_aligned_columns(output: &str, headers: &[&str], values: &[String]) {
    let lines = output.lines().collect::<Vec<_>>();
    assert!(
        lines.len() >= 3,
        "expected header, separator, and data: {output}"
    );
    for (header, value) in headers.iter().zip(values) {
        assert_eq!(
            lines[0].find(header),
            lines[2].find(value),
            "column {header} is not aligned:\n{output}"
        );
    }
}
