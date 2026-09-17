use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};
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
