use assert_cmd::Command;
use chrono::{Duration, Utc};
use predicates::prelude::*;
use stoker::domain::JobState;
use stoker::domain::flow::{DependencyMode, ExecutionMode, FlowRunState, ScheduleSpec};
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

fn new_job(root: &std::path::Path, name: &str) -> NewJob {
    NewJob {
        name: name.into(),
        user: "tester".into(),
        description: None,
        cwd: root.to_path_buf(),
        command: vec!["echo".into(), name.into()],
    }
}

fn create_committed_flow(store: &Store, root: &std::path::Path, flow_id: &str) {
    store
        .create_flow(
            flow_id.into(),
            flow_id.into(),
            "tester".into(),
            ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(2),
            },
        )
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow_id.into(),
            task_id: "root".into(),
            name: "root".into(),
            cwd: root.to_string_lossy().into_owned(),
            command: "echo flow".into(),
            retry: 0,
            dependencies: vec![],
            depend_mode: DependencyMode::All,
        })
        .unwrap();
    store.commit_flow(flow_id).unwrap();
}

fn lock_and_set_mode(home: &std::path::Path, cwd: &std::path::Path, mode: &str) {
    cli(home, cwd).args(["queue", "lock"]).assert().success();
    cli(home, cwd)
        .args(["mode", "set", mode])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Mode set to {mode}; queue remains locked."
        )));
}

#[test]
fn mode_set_requires_an_explicit_lock_and_rejects_active_work_in_both_modes() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");

    cli(&home, directory.path())
        .args(["mode", "set", "scheduled"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "queue is unlocked; run 'stoker queue lock' first",
        ));
    let store = Store::open(home.join("stoker.db")).unwrap();
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Serial);
    assert!(!store.queue_locked().unwrap());

    let serial_id = store
        .create_job(new_job(directory.path(), "active-serial"))
        .unwrap();
    store.commit_job(serial_id).unwrap();
    assert_eq!(store.claim_next().unwrap().unwrap().id, serial_id);
    lock_and_set_mode_fails_while_active(&home, directory.path(), "scheduled");
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Serial);
    assert!(store.queue_locked().unwrap());

    store.finish(serial_id, Some(0), None).unwrap();
    cli(&home, directory.path())
        .args(["mode", "set", "scheduled"])
        .assert()
        .success();
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Scheduled);
    assert!(store.queue_locked().unwrap());
    store.unlock_queue().unwrap();

    create_committed_flow(&store, directory.path(), "active-flow");
    let run = store
        .create_flow_run("active-flow", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();
    lock_and_set_mode_fails_while_active(&home, directory.path(), "serial");
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Scheduled);
    assert!(store.queue_locked().unwrap());

    let finished = store
        .finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(finished.run_id, run.run_id);
    assert_eq!(finished.state, FlowRunState::Succeeded);
    cli(&home, directory.path())
        .args(["mode", "set", "serial"])
        .assert()
        .success();
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Serial);
    assert!(store.queue_locked().unwrap());
}

fn lock_and_set_mode_fails_while_active(home: &std::path::Path, cwd: &std::path::Path, mode: &str) {
    cli(home, cwd).args(["queue", "lock"]).assert().success();
    cli(home, cwd)
        .args(["mode", "set", mode])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot change mode while an execution is active or cleanup is incomplete",
        ));
}

#[test]
fn serial_and_scheduled_records_survive_repeated_mode_switches() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();

    let completed_serial = store
        .create_job(new_job(directory.path(), "completed-serial"))
        .unwrap();
    store.commit_job(completed_serial).unwrap();
    assert_eq!(store.claim_next().unwrap().unwrap().id, completed_serial);
    store.finish(completed_serial, Some(0), None).unwrap();
    let queued_serial = store
        .create_job(new_job(directory.path(), "queued-serial"))
        .unwrap();
    store.commit_job(queued_serial).unwrap();
    let serial_log = home
        .join("runs")
        .join(completed_serial.to_string())
        .join("stdout.log");
    std::fs::create_dir_all(serial_log.parent().unwrap()).unwrap();
    std::fs::write(&serial_log, b"serial history\n").unwrap();

    lock_and_set_mode(&home, directory.path(), "scheduled");
    assert!(store.queue_locked().unwrap());
    store.unlock_queue().unwrap();
    create_committed_flow(&store, directory.path(), "scheduled-history");
    let scheduled_run = store
        .create_flow_run("scheduled-history", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();
    store
        .finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    let scheduled_log = home
        .join("runs")
        .join("flows")
        .join(scheduled_run.run_id.to_string())
        .join("root")
        .join("attempt-1")
        .join("stdout.log");
    std::fs::create_dir_all(scheduled_log.parent().unwrap()).unwrap();
    std::fs::write(&scheduled_log, b"scheduled history\n").unwrap();
    let occurrence_ids = store
        .list_occurrences("scheduled-history")
        .unwrap()
        .into_iter()
        .map(|item| item.occurrence_id)
        .collect::<Vec<_>>();

    lock_and_set_mode(&home, directory.path(), "serial");
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Serial);
    assert_eq!(
        store.get_job(completed_serial).unwrap().state,
        JobState::Succeeded
    );
    assert_eq!(
        store.get_job(queued_serial).unwrap().state,
        JobState::Queued
    );
    assert_eq!(
        std::fs::read_to_string(&serial_log).unwrap(),
        "serial history\n"
    );
    assert!(store.get_flow("scheduled-history").unwrap().committed);
    assert_eq!(
        store.get_flow_run(scheduled_run.run_id).unwrap().state,
        FlowRunState::Succeeded
    );
    assert_eq!(
        store
            .list_occurrences("scheduled-history")
            .unwrap()
            .into_iter()
            .map(|item| item.occurrence_id)
            .collect::<Vec<_>>(),
        occurrence_ids
    );
    assert_eq!(
        std::fs::read_to_string(&scheduled_log).unwrap(),
        "scheduled history\n"
    );

    store.unlock_queue().unwrap();
    lock_and_set_mode(&home, directory.path(), "scheduled");
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Scheduled);
    assert_eq!(
        store.get_job(queued_serial).unwrap().state,
        JobState::Queued
    );
    assert_eq!(
        store.list_flow_runs("scheduled-history").unwrap()[0].run_id,
        scheduled_run.run_id
    );
    assert!(serial_log.is_file());
    assert!(scheduled_log.is_file());
}

#[test]
fn recovery_fence_blocks_mode_changes_until_the_recovering_run_is_reconciled() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let store = Store::open(home.join("stoker.db")).unwrap();
    lock_and_set_mode(&home, directory.path(), "scheduled");
    store.unlock_queue().unwrap();
    create_committed_flow(&store, directory.path(), "recovering-flow");
    let run = store
        .create_flow_run("recovering-flow", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();

    assert!(store.recover_runtime_jobs().unwrap());
    assert!(store.queue_locked().unwrap());
    assert!(store.queue_recovery_fence().unwrap());
    assert_eq!(
        store.get_flow_run(run.run_id).unwrap().state,
        FlowRunState::Recovering
    );
    cli(&home, directory.path())
        .args(["mode", "set", "serial"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot change mode while an execution is active or cleanup is incomplete",
        ));
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Scheduled);

    store.reconcile_recovery(run.run_id, true).unwrap();
    assert!(!store.queue_recovery_fence().unwrap());
    cli(&home, directory.path())
        .args(["mode", "set", "serial"])
        .assert()
        .success();
    assert_eq!(store.current_mode().unwrap(), ExecutionMode::Serial);
    assert!(store.queue_locked().unwrap());
}
