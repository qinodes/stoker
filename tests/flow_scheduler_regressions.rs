use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};
use std::sync::{Arc, Barrier};
use stoker::domain::flow::{
    Dependency, DependencyMode, DependencyStatus, ExecutionMode, FlowRunState, OccurrenceState,
    ScheduleSpec, TaskRunState, parse_once,
};
use stoker::store::{FlowAttemptResult, FlowTaskInput};
use stoker::{NewJob, Store, StoreError};
use uuid::Uuid;

fn add_task(
    store: &Store,
    root: &std::path::Path,
    flow_id: &str,
    task_id: &str,
    dependencies: Vec<Dependency>,
    depend_mode: DependencyMode,
    retry: u32,
) {
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow_id.into(),
            task_id: task_id.into(),
            name: task_id.into(),
            cwd: root.to_string_lossy().into_owned(),
            command: "echo regression".into(),
            retry,
            dependencies,
            depend_mode,
        })
        .unwrap();
}

fn create_scheduled_flow(
    store: &Store,
    root: &std::path::Path,
    flow_id: &str,
    schedule: ScheduleSpec,
) {
    store
        .create_flow(flow_id.into(), flow_id.into(), "tester".into(), schedule)
        .unwrap();
    add_task(store, root, flow_id, "root", vec![], DependencyMode::All, 0);
    store.commit_flow(flow_id).unwrap();
}

fn use_scheduled_mode(store: &Store) {
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();
}

fn future_once() -> ScheduleSpec {
    ScheduleSpec::Once {
        at: Utc::now() + Duration::hours(2),
    }
}

fn create_runnable_flow(store: &Store, root: &std::path::Path, flow_id: &str) {
    create_scheduled_flow(store, root, flow_id, future_once());
    use_scheduled_mode(store);
}

fn next_minute() -> (NaiveTime, DateTime<Utc>) {
    let now = Utc::now();
    let due = (now + Duration::minutes(1))
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();
    (due.time(), due)
}

#[test]
fn user_defined_flow_creation_is_always_scheduled() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let flow = store
        .create_flow(
            "scheduled-only".into(),
            "Scheduled".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    assert_eq!(flow.mode, ExecutionMode::Scheduled);
}

#[test]
fn scheduled_job_listing_does_not_reenter_the_store_lock() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let id = store
        .create_job(NewJob {
            name: "scheduled".into(),
            user: "tester".into(),
            description: None,
            cwd: directory.path().into(),
            command: vec!["echo".into(), "ok".into()],
        })
        .unwrap();
    store
        .configure_standalone(id, ExecutionMode::Scheduled, Some(future_once()), 0)
        .unwrap();

    let jobs = store
        .list_jobs_for_mode(None, None, Some(ExecutionMode::Scheduled))
        .unwrap();

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, id);
}

#[test]
fn manual_run_obeys_queue_lock_and_request_id_is_idempotent_while_active() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_scheduled_flow(&store, directory.path(), "manual-gates", future_once());
    use_scheduled_mode(&store);
    store.lock_queue().unwrap();
    assert!(matches!(
        store.create_flow_run("manual-gates", "MANUAL", false, None),
        Err(StoreError::QueueLocked)
    ));
    store.unlock_queue().unwrap();

    let request_id = Uuid::new_v4();
    let first = store
        .create_flow_run("manual-gates", "MANUAL", false, Some(request_id))
        .unwrap();
    let replay = store
        .create_flow_run("manual-gates", "MANUAL", false, Some(request_id))
        .unwrap();

    assert_eq!(replay.run_id, first.run_id);
}

#[test]
fn manual_run_rejects_full_scheduled_process_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    for id in ["capacity-a", "capacity-b"] {
        create_scheduled_flow(&store, directory.path(), id, future_once());
    }
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.set_scheduled_concurrency(1).unwrap();
    store.unlock_queue().unwrap();
    store
        .create_flow_run("capacity-a", "MANUAL", false, None)
        .unwrap();
    let first = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store.mark_flow_attempt_running(first.attempt_id).unwrap();

    let error = store
        .create_flow_run("capacity-b", "MANUAL", false, None)
        .unwrap_err();
    assert!(error.to_string().contains("capacity is full"));
}

#[test]
fn disabled_manual_run_is_claimable_but_freeze_pauses_intake() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_scheduled_flow(&store, directory.path(), "disabled-manual", future_once());
    use_scheduled_mode(&store);
    store.disable_flow("disabled-manual", false).unwrap();
    let run = store
        .create_flow_run("disabled-manual", "MANUAL", false, None)
        .unwrap();
    assert_eq!(
        store.claim_flow_task(Utc::now()).unwrap().unwrap().run_id,
        run.run_id
    );

    create_scheduled_flow(&store, directory.path(), "frozen-intake", future_once());
    let frozen_run = store
        .create_flow_run("frozen-intake", "MANUAL", false, None)
        .unwrap();
    store.freeze_flow("frozen-intake").unwrap();
    assert!(store.claim_flow_task(Utc::now()).unwrap().is_none());
    assert_eq!(
        store.get_flow_run(frozen_run.run_id).unwrap().tasks[0].state,
        TaskRunState::Ready
    );
}

#[test]
fn first_root_has_a_spawn_acknowledgement_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    store
        .create_flow(
            "root-barrier".into(),
            "root-barrier".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    for task in ["a", "b"] {
        add_task(
            &store,
            directory.path(),
            "root-barrier",
            task,
            vec![],
            DependencyMode::All,
            0,
        );
    }
    store.commit_flow("root-barrier").unwrap();
    use_scheduled_mode(&store);
    store
        .create_flow_run("root-barrier", "MANUAL", false, None)
        .unwrap();
    let first = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    assert!(store.claim_flow_task(Utc::now()).unwrap().is_none());
    store.mark_flow_attempt_running(first.attempt_id).unwrap();
    assert!(store.claim_flow_task(Utc::now()).unwrap().is_some());
}

#[test]
fn first_root_spawn_failure_is_not_retried() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    store
        .create_flow(
            "spawn-failure".into(),
            "spawn-failure".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    add_task(
        &store,
        directory.path(),
        "spawn-failure",
        "root",
        vec![],
        DependencyMode::All,
        2,
    );
    store.commit_flow("spawn-failure").unwrap();
    use_scheduled_mode(&store);
    let run = store
        .create_flow_run("spawn-failure", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let finished = store
        .finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Failed {
                exit_code: None,
                kind: "SPAWN".into(),
                detail: "not found".into(),
            },
        )
        .unwrap();
    assert_eq!(finished.run_id, run.run_id);
    assert_eq!(finished.state, FlowRunState::FailedToStart);
    assert_ne!(finished.tasks[0].state, TaskRunState::RetryWait);
}

#[test]
fn active_task_cancellation_persists_an_intent() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_runnable_flow(&store, directory.path(), "cancel-active");
    let run = store
        .create_flow_run("cancel-active", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();

    let cancelling = store
        .cancel_flow_task("cancel-active", "root", run.run_id)
        .unwrap();

    assert_eq!(cancelling.tasks[0].state, TaskRunState::Cancelling);
    assert!(cancelling.tasks[0].cancel_requested);
    assert!(
        store
            .flow_attempt_cancel_requested(execution.attempt_id)
            .unwrap()
    );
}

#[test]
fn future_removal_cascades_without_promoting_children_to_roots() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    store
        .create_flow(
            "remove-cascade".into(),
            "remove-cascade".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    add_task(
        &store,
        directory.path(),
        "remove-cascade",
        "root",
        vec![],
        DependencyMode::All,
        0,
    );
    add_task(
        &store,
        directory.path(),
        "remove-cascade",
        "child",
        vec![Dependency {
            upstream_task_id: "root".into(),
            status: DependencyStatus::Succeeded,
        }],
        DependencyMode::All,
        0,
    );
    add_task(
        &store,
        directory.path(),
        "remove-cascade",
        "keep",
        vec![],
        DependencyMode::All,
        0,
    );
    store.commit_flow("remove-cascade").unwrap();
    store.freeze_flow("remove-cascade").unwrap();

    let draft = store
        .remove_flow_task("remove-cascade", "root", "future", None, None)
        .unwrap();

    assert_eq!(draft.tasks.len(), 1);
    assert_eq!(draft.tasks[0].task_id, "keep");
}

#[test]
fn both_scope_validation_is_atomic_and_current_scope_requires_freeze() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_runnable_flow(&store, directory.path(), "atomic-edit");
    let run = store
        .create_flow_run("atomic-edit", "MANUAL", false, None)
        .unwrap();
    assert!(
        store
            .remove_flow_task("atomic-edit", "root", "current", Some(run.run_id), None)
            .is_err()
    );
    store.freeze_flow("atomic-edit").unwrap();
    assert!(
        store
            .remove_flow_task("atomic-edit", "root", "both", Some(run.run_id), None)
            .is_err()
    );
    assert_eq!(
        store.get_flow_run(run.run_id).unwrap().tasks[0].state,
        TaskRunState::Ready
    );
}

#[test]
fn daily_due_records_disabled_and_workspace_mode_skip_reasons() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let (time, due) = next_minute();
    let schedule = || ScheduleSpec::Daily {
        time,
        timezone: "UTC".into(),
    };
    create_scheduled_flow(&store, directory.path(), "daily-disabled", schedule());
    store.disable_flow("daily-disabled", false).unwrap();
    store.claim_flow_task(due + Duration::seconds(1)).unwrap();
    let disabled = store.list_occurrences("daily-disabled").unwrap();
    assert_eq!(disabled.len(), 1);
    assert_eq!(disabled[0].state, OccurrenceState::Skipped);
    assert_eq!(disabled[0].reason.as_deref(), Some("SKIPPED_DISABLED"));

    create_scheduled_flow(&store, directory.path(), "daily-mode", schedule());
    store.claim_flow_task(due + Duration::seconds(1)).unwrap();
    let wrong_mode = store.list_occurrences("daily-mode").unwrap();
    assert_eq!(wrong_mode.len(), 1);
    assert_eq!(wrong_mode[0].state, OccurrenceState::Skipped);
    assert_eq!(wrong_mode[0].reason.as_deref(), Some("SKIPPED_MODE"));
}

#[test]
fn skip_next_is_replaced_on_spawn_acknowledgement() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_scheduled_flow(&store, directory.path(), "skip-next", future_once());
    use_scheduled_mode(&store);
    store
        .create_flow_run("skip-next", "MANUAL", true, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();

    assert_eq!(
        store.list_occurrences("skip-next").unwrap()[0].state,
        OccurrenceState::Replaced
    );
}

#[test]
fn non_skip_manual_once_run_does_not_make_the_future_schedule_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_scheduled_flow(&store, directory.path(), "extra-manual", future_once());
    use_scheduled_mode(&store);
    store
        .create_flow_run("extra-manual", "MANUAL", false, None)
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
    store.freeze_flow("extra-manual").unwrap();

    assert!(
        store
            .set_flow_schedule_draft(
                "extra-manual",
                ScheduleSpec::Once {
                    at: Utc::now() + Duration::hours(3),
                },
                None,
            )
            .is_ok()
    );
}

#[test]
fn expired_once_is_settled_before_schedule_edit_validation() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_scheduled_flow(
        &store,
        directory.path(),
        "expired-edit",
        ScheduleSpec::Once {
            at: Utc::now() - Duration::hours(25),
        },
    );
    store.freeze_flow("expired-edit").unwrap();

    assert!(
        store
            .set_flow_schedule_draft(
                "expired-edit",
                ScheduleSpec::Once {
                    at: Utc::now() + Duration::hours(1),
                },
                None,
            )
            .is_err()
    );
}

#[test]
fn unlaunched_manual_run_is_not_misclassified_as_restart_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_runnable_flow(&store, directory.path(), "offline-manual");
    let run = store
        .create_flow_run("offline-manual", "MANUAL", false, None)
        .unwrap();

    assert!(!store.recover_runtime_jobs().unwrap());
    assert_eq!(
        store.get_flow_run(run.run_id).unwrap().state,
        FlowRunState::Starting
    );
    assert!(!store.queue_recovery_fence().unwrap());
}

#[test]
fn flow_names_allow_colons_dependencies_cannot_contradict_and_leap_seconds_fail() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    store
        .create_flow(
            "display-name".into(),
            "Build: nightly".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    add_task(
        &store,
        directory.path(),
        "display-name",
        "a",
        vec![],
        DependencyMode::All,
        0,
    );
    add_task(
        &store,
        directory.path(),
        "display-name",
        "b",
        vec![
            Dependency {
                upstream_task_id: "a".into(),
                status: DependencyStatus::Succeeded,
            },
            Dependency {
                upstream_task_id: "a".into(),
                status: DependencyStatus::Failed,
            },
        ],
        DependencyMode::Any,
        0,
    );
    let error = store.commit_flow("display-name").unwrap_err();
    assert!(error.to_string().contains("contradictory"));
    assert!(parse_once("2016-12-31T23:59:60Z").is_err());
}

#[test]
fn task_run_transitions_to_running_with_its_attempt() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    create_runnable_flow(&store, directory.path(), "task-running");
    let run = store
        .create_flow_run("task-running", "MANUAL", false, None)
        .unwrap();
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();
    assert_eq!(
        store.get_flow_run(run.run_id).unwrap().tasks[0].state,
        TaskRunState::Running
    );
}

#[test]
fn concurrent_claimers_create_only_one_run_and_attempt_for_one_occurrence() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let setup = Store::open(&database).unwrap();
    create_scheduled_flow(
        &setup,
        directory.path(),
        "claim-race",
        ScheduleSpec::Once {
            at: Utc::now() - Duration::seconds(1),
        },
    );
    use_scheduled_mode(&setup);
    let first_store = Arc::new(Store::open(&database).unwrap());
    let second_store = Arc::new(Store::open(&database).unwrap());
    let barrier = Arc::new(Barrier::new(3));
    let now = Utc::now();

    let first_barrier = Arc::clone(&barrier);
    let first = Arc::clone(&first_store);
    let first_handle = std::thread::spawn(move || {
        first_barrier.wait();
        first.claim_flow_task(now)
    });
    let second_barrier = Arc::clone(&barrier);
    let second = Arc::clone(&second_store);
    let second_handle = std::thread::spawn(move || {
        second_barrier.wait();
        second.claim_flow_task(now)
    });
    barrier.wait();

    let results = [first_handle.join().unwrap(), second_handle.join().unwrap()];
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(
        results
            .iter()
            .filter(|result| result.as_ref().unwrap().is_some())
            .count(),
        1
    );
    assert_eq!(setup.list_flow_runs("claim-race").unwrap().len(), 1);
    assert_eq!(setup.list_occurrences("claim-race").unwrap().len(), 1);
    assert_eq!(
        setup
            .get_flow_run(setup.list_flow_runs("claim-race").unwrap()[0].run_id)
            .unwrap()
            .tasks[0]
            .attempt_count,
        1
    );
}

#[test]
fn concurrent_manual_requests_with_one_request_id_return_one_durable_run() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let setup = Store::open(&database).unwrap();
    create_runnable_flow(&setup, directory.path(), "request-race");
    let first_store = Arc::new(Store::open(&database).unwrap());
    let second_store = Arc::new(Store::open(&database).unwrap());
    let barrier = Arc::new(Barrier::new(3));
    let request_id = Uuid::new_v4();

    let first_barrier = Arc::clone(&barrier);
    let first = Arc::clone(&first_store);
    let first_handle = std::thread::spawn(move || {
        first_barrier.wait();
        first.create_flow_run("request-race", "MANUAL", false, Some(request_id))
    });
    let second_barrier = Arc::clone(&barrier);
    let second = Arc::clone(&second_store);
    let second_handle = std::thread::spawn(move || {
        second_barrier.wait();
        second.create_flow_run("request-race", "MANUAL", false, Some(request_id))
    });
    barrier.wait();

    let first = first_handle.join().unwrap().unwrap();
    let second = second_handle.join().unwrap().unwrap();
    assert_eq!(first.run_id, second.run_id);
    assert_eq!(setup.list_flow_runs("request-race").unwrap().len(), 1);
    assert_eq!(
        setup.manual_request(request_id).unwrap().run_id,
        Some(first.run_id)
    );
}

#[test]
fn concurrent_freeze_and_claim_never_lose_an_authorized_execution() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let setup = Store::open(&database).unwrap();
    create_runnable_flow(&setup, directory.path(), "freeze-race");
    let run = setup
        .create_flow_run("freeze-race", "MANUAL", false, None)
        .unwrap();
    let freeze_store = Arc::new(Store::open(&database).unwrap());
    let claim_store = Arc::new(Store::open(&database).unwrap());
    let barrier = Arc::new(Barrier::new(3));

    let freeze_barrier = Arc::clone(&barrier);
    let freeze = Arc::clone(&freeze_store);
    let freeze_handle = std::thread::spawn(move || {
        freeze_barrier.wait();
        freeze.freeze_flow("freeze-race")
    });
    let claim_barrier = Arc::clone(&barrier);
    let claim = Arc::clone(&claim_store);
    let claim_handle = std::thread::spawn(move || {
        claim_barrier.wait();
        claim.claim_flow_task(Utc::now())
    });
    barrier.wait();

    freeze_handle.join().unwrap().unwrap();
    let claimed = claim_handle.join().unwrap().unwrap();
    assert!(setup.get_flow("freeze-race").unwrap().frozen);
    let task = &setup.get_flow_run(run.run_id).unwrap().tasks[0];
    match claimed {
        Some(execution) => {
            assert_eq!(execution.run_id, run.run_id);
            assert_eq!(task.state, TaskRunState::Starting);
            assert_eq!(task.attempt_count, 1);
        }
        None => {
            assert_eq!(task.state, TaskRunState::Ready);
            assert_eq!(task.attempt_count, 0);
        }
    }
}

#[test]
fn concurrent_apply_and_claim_publish_the_whole_draft_before_intake() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let setup = Store::open(&database).unwrap();
    create_runnable_flow(&setup, directory.path(), "apply-race");
    let run = setup
        .create_flow_run("apply-race", "MANUAL", false, None)
        .unwrap();
    setup.freeze_flow("apply-race").unwrap();
    setup
        .set_flow_task_draft(
            "apply-race",
            "root",
            Some("echo applied"),
            None,
            Some(2),
            None,
            None,
            None,
        )
        .unwrap();
    let apply_store = Arc::new(Store::open(&database).unwrap());
    let claim_store = Arc::new(Store::open(&database).unwrap());
    let barrier = Arc::new(Barrier::new(3));

    let apply_barrier = Arc::clone(&barrier);
    let apply = Arc::clone(&apply_store);
    let apply_handle = std::thread::spawn(move || {
        apply_barrier.wait();
        apply.unfreeze_flow("apply-race", Some(1))
    });
    let claim_barrier = Arc::clone(&barrier);
    let claim = Arc::clone(&claim_store);
    let claim_handle = std::thread::spawn(move || {
        claim_barrier.wait();
        claim.claim_flow_task(Utc::now())
    });
    barrier.wait();

    apply_handle.join().unwrap().unwrap();
    let raced_claim = claim_handle.join().unwrap().unwrap();
    let applied = setup.get_flow("apply-race").unwrap();
    assert!(!applied.frozen);
    assert_eq!(applied.tasks[0].command, "echo applied");
    assert_eq!(applied.tasks[0].retry, 2);
    let execution = match raced_claim {
        Some(execution) => execution,
        None => setup.claim_flow_task(Utc::now()).unwrap().unwrap(),
    };
    assert_eq!(execution.run_id, run.run_id);
    assert_eq!(
        setup.get_flow_run(run.run_id).unwrap().tasks[0].attempt_count,
        1
    );
}

#[test]
fn concurrent_cancel_and_finish_leave_one_valid_terminal_flow_state() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let setup = Store::open(&database).unwrap();
    create_runnable_flow(&setup, directory.path(), "cancel-finish-race");
    let run = setup
        .create_flow_run("cancel-finish-race", "MANUAL", false, None)
        .unwrap();
    let execution = setup.claim_flow_task(Utc::now()).unwrap().unwrap();
    setup
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();
    let cancel_store = Arc::new(Store::open(&database).unwrap());
    let finish_store = Arc::new(Store::open(&database).unwrap());
    let barrier = Arc::new(Barrier::new(3));

    let cancel_barrier = Arc::clone(&barrier);
    let cancel = Arc::clone(&cancel_store);
    let cancel_handle = std::thread::spawn(move || {
        cancel_barrier.wait();
        cancel.cancel_flow_task("cancel-finish-race", "root", run.run_id)
    });
    let finish_barrier = Arc::clone(&barrier);
    let finish = Arc::clone(&finish_store);
    let finish_handle = std::thread::spawn(move || {
        finish_barrier.wait();
        finish.finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Succeeded { exit_code: 0 },
        )
    });
    barrier.wait();

    let cancel_result = cancel_handle.join().unwrap();
    let finish_result = finish_handle.join().unwrap();
    assert!(finish_result.is_ok());
    assert!(
        cancel_result.is_ok()
            || cancel_result
                .unwrap_err()
                .to_string()
                .contains("already terminal")
    );
    let final_run = setup.get_flow_run(run.run_id).unwrap();
    assert!(matches!(
        final_run.state,
        FlowRunState::Succeeded | FlowRunState::Cancelled
    ));
    assert!(final_run.state.is_terminal());
    assert!(final_run.tasks[0].state.is_terminal());
}

#[test]
fn concurrent_manual_and_automatic_start_never_overlap_one_flow() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let setup = Store::open(&database).unwrap();
    create_scheduled_flow(
        &setup,
        directory.path(),
        "manual-auto-race",
        ScheduleSpec::Once {
            at: Utc::now() - Duration::seconds(1),
        },
    );
    use_scheduled_mode(&setup);
    let manual_store = Arc::new(Store::open(&database).unwrap());
    let automatic_store = Arc::new(Store::open(&database).unwrap());
    let barrier = Arc::new(Barrier::new(3));

    let manual_barrier = Arc::clone(&barrier);
    let manual = Arc::clone(&manual_store);
    let manual_handle = std::thread::spawn(move || {
        manual_barrier.wait();
        manual.create_flow_run("manual-auto-race", "MANUAL", false, Some(Uuid::new_v4()))
    });
    let automatic_barrier = Arc::clone(&barrier);
    let automatic = Arc::clone(&automatic_store);
    let automatic_handle = std::thread::spawn(move || {
        automatic_barrier.wait();
        automatic.claim_flow_task(Utc::now())
    });
    barrier.wait();

    let manual_result = manual_handle.join().unwrap();
    let automatic_result = automatic_handle.join().unwrap();
    assert!(automatic_result.is_ok());
    assert!(
        manual_result.is_ok()
            || manual_result
                .unwrap_err()
                .to_string()
                .contains("already has an active run")
    );
    let runs = setup.list_flow_runs("manual-auto-race").unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs.iter().filter(|run| !run.state.is_terminal()).count(),
        1
    );
}
