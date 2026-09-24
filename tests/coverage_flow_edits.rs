use chrono::{Duration, Utc};
use stoker::domain::flow::{
    AttemptState, Dependency, DependencyMode, DependencyStatus, ExecutionMode, FlowRunState,
    OccurrenceState, ScheduleSpec,
};
use stoker::store::{FlowAttemptResult, FlowTaskInput};
use stoker::{Store, StoreError};

fn task(root: &std::path::Path, flow_id: &str, task_id: &str) -> FlowTaskInput {
    FlowTaskInput {
        flow_id: flow_id.into(),
        task_id: task_id.into(),
        name: task_id.into(),
        cwd: root.to_string_lossy().into_owned(),
        command: format!("echo {task_id}"),
        retry: 0,
        dependencies: vec![],
        depend_mode: DependencyMode::All,
    }
}

fn flow(store: &Store, root: &std::path::Path) {
    store
        .create_flow(
            "coverage-flow".into(),
            "Coverage flow".into(),
            "tester".into(),
            ScheduleSpec::Once {
                at: Utc::now() + Duration::hours(2),
            },
        )
        .unwrap();
    store
        .add_flow_task(task(root, "coverage-flow", "root"))
        .unwrap();
}

#[test]
fn task_additions_preserve_frozen_drafts_and_require_the_current_revision() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    flow(&store, directory.path());
    assert!(matches!(
        store.add_flow_task(task(directory.path(), "coverage-flow", "root")),
        Err(StoreError::InvalidData(message)) if message.contains("already exists")
    ));
    store.commit_flow("coverage-flow").unwrap();
    assert!(matches!(
        store.add_flow_task(task(directory.path(), "coverage-flow", "before-freeze")),
        Err(StoreError::InvalidData(message)) if message.contains("frozen")
    ));
    store.freeze_flow("coverage-flow").unwrap();
    let first = store
        .add_flow_task_with_revision(task(directory.path(), "coverage-flow", "second"), Some(0))
        .unwrap();
    assert!(first.has_draft);
    assert_eq!(first.draft_revision, 1);
    assert_eq!(first.tasks.len(), 2);
    assert!(matches!(
        store.add_flow_task_with_revision(task(directory.path(), "coverage-flow", "third"), None),
        Err(StoreError::InvalidData(message)) if message.contains("revision is required")
    ));
    assert!(matches!(
        store
            .add_flow_task_with_revision(task(directory.path(), "coverage-flow", "third"), Some(0)),
        Err(StoreError::DraftRevisionConflict {
            expected: 0,
            current: 1
        })
    ));
    let third = store
        .add_flow_task_with_revision(task(directory.path(), "coverage-flow", "third"), Some(1))
        .unwrap();
    assert_eq!(
        third
            .tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "second", "third"]
    );
}

#[test]
fn task_draft_edits_validate_state_revision_and_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    flow(&store, directory.path());
    assert!(matches!(
        store.set_flow_task_draft("coverage-flow", "root", Some("echo edited"), None, None, None, None, None),
        Err(StoreError::InvalidData(message)) if message.contains("committed and frozen")
    ));
    store.commit_flow("coverage-flow").unwrap();
    store.freeze_flow("coverage-flow").unwrap();
    let first = store
        .set_flow_task_draft(
            "coverage-flow",
            "root",
            Some("echo edited"),
            Some(directory.path()),
            Some(2),
            Some(vec![]),
            Some(DependencyMode::Any),
            Some(0),
        )
        .unwrap();
    assert_eq!(first.tasks[0].command, "echo edited");
    assert_eq!(first.tasks[0].retry, 2);
    assert_eq!(first.tasks[0].depend_mode, DependencyMode::Any);
    assert!(matches!(
        store.set_flow_task_draft("coverage-flow", "root", None, None, None, None, None, None),
        Err(StoreError::InvalidData(message)) if message.contains("revision is required")
    ));
    assert!(matches!(
        store.set_flow_task_draft(
            "coverage-flow",
            "root",
            None,
            None,
            None,
            None,
            None,
            Some(0)
        ),
        Err(StoreError::DraftRevisionConflict {
            expected: 0,
            current: 1
        })
    ));
    assert!(matches!(
        store.set_flow_task_draft("coverage-flow", "missing", None, None, None, None, None, Some(1)),
        Err(StoreError::InvalidData(message)) if message.contains("does not exist")
    ));
    assert!(matches!(
        store.set_flow_task_draft(
            "coverage-flow",
            "root",
            None,
            None,
            None,
            Some(vec![Dependency {
                upstream_task_id: "missing".into(),
                status: DependencyStatus::Succeeded
            }]),
            None,
            Some(1)
        ),
        Err(StoreError::InvalidData(_))
    ));
    assert_eq!(store.get_flow("coverage-flow").unwrap().draft_revision, 1);
}

#[test]
fn future_removal_rejects_invalid_edits_and_removes_dependent_branch() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    flow(&store, directory.path());
    let mut child = task(directory.path(), "coverage-flow", "child");
    child.dependencies = vec![Dependency {
        upstream_task_id: "root".into(),
        status: DependencyStatus::Succeeded,
    }];
    store.add_flow_task(child).unwrap();
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "child", "invalid", None, None),
        Err(StoreError::InvalidData(message)) if message.contains("scope")
    ));
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "child", "current", None, None),
        Err(StoreError::InvalidData(message)) if message.contains("require --run")
    ));
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "child", "future", None, None),
        Err(StoreError::InvalidData(message)) if message.contains("committed and frozen")
    ));
    store.commit_flow("coverage-flow").unwrap();
    store.freeze_flow("coverage-flow").unwrap();
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "missing", "future", None, Some(0)),
        Err(StoreError::InvalidData(message)) if message.contains("does not exist")
    ));
    let child_removed = store
        .remove_flow_task("coverage-flow", "child", "future", None, Some(0))
        .unwrap();
    assert_eq!(child_removed.tasks.len(), 1);
    assert_eq!(child_removed.tasks[0].task_id, "root");
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "root", "future", None, None),
        Err(StoreError::InvalidData(message)) if message.contains("revision is required")
    ));
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "root", "future", None, Some(0)),
        Err(StoreError::DraftRevisionConflict {
            expected: 0,
            current: 1
        })
    ));
    assert!(matches!(
        store.remove_flow_task("coverage-flow", "root", "future", None, Some(1)),
        Err(StoreError::InvalidData(message)) if message.contains("cannot be empty")
    ));
}

#[test]
fn schedule_draft_rejects_family_changes_stale_revisions_and_terminal_once_occurrences() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stoker.db");
    let store = Store::open(&database).unwrap();
    flow(&store, directory.path());
    let next = ScheduleSpec::Once {
        at: Utc::now() + Duration::hours(3),
    };
    assert!(matches!(
        store.set_flow_schedule_draft("coverage-flow", next.clone(), None),
        Err(StoreError::InvalidData(message)) if message.contains("frozen")
    ));
    store.commit_flow("coverage-flow").unwrap();
    store.freeze_flow("coverage-flow").unwrap();
    assert!(matches!(
        store.set_flow_schedule_draft("coverage-flow", ScheduleSpec::Daily { time: chrono::NaiveTime::from_hms_opt(8, 30, 0).unwrap(), timezone: "UTC".into() }, Some(0)),
        Err(StoreError::InvalidData(message)) if message.contains("switching")
    ));
    assert!(matches!(
        store.set_flow_schedule_draft("coverage-flow", ScheduleSpec::Once { at: Utc::now() - Duration::hours(1) }, Some(0)),
        Err(StoreError::InvalidData(message)) if message.contains("future")
    ));
    let edited = store
        .set_flow_schedule_draft("coverage-flow", next.clone(), Some(0))
        .unwrap();
    assert_eq!(edited.schedule, Some(next.clone()));
    assert_eq!(edited.draft_revision, 1);
    assert!(matches!(
        store.set_flow_schedule_draft("coverage-flow", next.clone(), None),
        Err(StoreError::InvalidData(message)) if message.contains("revision is required")
    ));
    assert!(matches!(
        store.set_flow_schedule_draft("coverage-flow", next.clone(), Some(0)),
        Err(StoreError::DraftRevisionConflict {
            expected: 0,
            current: 1
        })
    ));
    let unchanged = store
        .set_flow_schedule_draft("coverage-flow", next.clone(), Some(1))
        .unwrap();
    assert_eq!(unchanged.draft_revision, 1);

    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute(
            "UPDATE occurrences SET state = 'SKIPPED' WHERE flow_id = 'coverage-flow'",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.set_flow_schedule_draft("coverage-flow", ScheduleSpec::Once { at: Utc::now() + Duration::hours(4) }, Some(1)),
        Err(StoreError::InvalidData(message)) if message.contains("terminal one-time")
    ));
}

#[test]
fn manual_run_tracks_active_counts_and_request_idempotency() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    flow(&store, directory.path());
    assert!(matches!(
        store.create_flow_run("coverage-flow", "MANUAL", false, None),
        Err(StoreError::InvalidData(message)) if message.contains("committed")
    ));
    store.commit_flow("coverage-flow").unwrap();
    assert!(matches!(
        store.create_flow_run("coverage-flow", "MANUAL", false, None),
        Err(StoreError::InvalidData(message)) if message.contains("workspace mode")
    ));
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();
    let request_id = uuid::Uuid::new_v4();
    let run = store
        .create_flow_run("coverage-flow", "MANUAL", false, Some(request_id))
        .unwrap();
    let repeat = store
        .create_flow_run("coverage-flow", "MANUAL", false, Some(request_id))
        .unwrap();
    assert_eq!(repeat.run_id, run.run_id);
    assert!(matches!(
        store.create_flow_run("coverage-flow", "MANUAL", true, Some(request_id)),
        Err(StoreError::InvalidData(message)) if message.contains("different parameters")
    ));
    assert_eq!(store.flow_active_count().unwrap(), 1);
    assert_eq!(store.flow_active_attempt_count().unwrap(), 0);
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    assert_eq!(execution.run_id, run.run_id);
    assert_eq!(store.flow_active_attempt_count().unwrap(), 1);
    store
        .mark_flow_attempt_running(execution.attempt_id)
        .unwrap();
    let finished = store
        .finish_flow_attempt(
            execution.attempt_id,
            FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert!(finished.state.is_terminal());
    assert_eq!(store.flow_active_count().unwrap(), 0);
    assert_eq!(store.flow_active_attempt_count().unwrap(), 0);
}

#[test]
fn frozen_daily_flow_discards_stale_drafts_and_applies_a_new_schedule_generation() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    store
        .create_flow(
            "daily-edit".into(),
            "Daily edit".into(),
            "tester".into(),
            ScheduleSpec::Daily {
                time: chrono::NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
                timezone: "Asia/Tokyo".into(),
            },
        )
        .unwrap();
    store
        .add_flow_task(task(directory.path(), "daily-edit", "root"))
        .unwrap();
    store.commit_flow("daily-edit").unwrap();
    assert!(
        matches!(store.discard_flow_draft("daily-edit", 0), Err(StoreError::InvalidData(message)) if message.contains("not frozen"))
    );
    store.freeze_flow("daily-edit").unwrap();
    assert!(
        matches!(store.discard_flow_draft("daily-edit", 0), Err(StoreError::InvalidData(message)) if message.contains("no draft"))
    );
    let edit = store
        .set_flow_task_draft(
            "daily-edit",
            "root",
            Some("echo changed"),
            None,
            None,
            None,
            None,
            Some(0),
        )
        .unwrap();
    assert!(matches!(
        store.discard_flow_draft("daily-edit", 0),
        Err(StoreError::DraftRevisionConflict {
            expected: 0,
            current: 1
        })
    ));
    let discarded = store
        .discard_flow_draft("daily-edit", edit.draft_revision)
        .unwrap();
    assert!(!discarded.has_draft);
    let next = ScheduleSpec::Daily {
        time: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        timezone: "Asia/Tokyo".into(),
    };
    let edited = store
        .set_flow_schedule_draft("daily-edit", next.clone(), Some(discarded.draft_revision))
        .unwrap();
    let applied = store
        .unfreeze_flow("daily-edit", Some(edited.draft_revision))
        .unwrap();
    assert_eq!(applied.schedule, Some(next));
    assert_eq!(applied.schedule_generation, 2);
    assert!(!applied.frozen);
    assert!(!applied.has_draft);
}

#[test]
fn cancelling_all_manual_runs_settles_every_pending_task_and_request() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    for flow_id in ["cancel-one", "cancel-two"] {
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
            .add_flow_task(task(directory.path(), flow_id, "root"))
            .unwrap();
        store.commit_flow(flow_id).unwrap();
    }
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();
    let first_request = uuid::Uuid::new_v4();
    let first = store
        .create_flow_run("cancel-one", "MANUAL", false, Some(first_request))
        .unwrap();
    let second = store
        .create_flow_run("cancel-two", "MANUAL", false, None)
        .unwrap();
    let cancelled = store.cancel_all_flow_runs().unwrap();
    assert_eq!(cancelled.len(), 2);
    assert!(cancelled.iter().all(|run| run.state.is_terminal()));
    assert!(
        cancelled
            .iter()
            .all(|run| run.tasks.iter().all(|task| task.state.is_terminal()))
    );
    assert_eq!(
        store.get_flow_run(first.run_id).unwrap().state,
        cancelled
            .iter()
            .find(|run| run.run_id == first.run_id)
            .unwrap()
            .state
    );
    assert!(cancelled.iter().any(|run| run.run_id == second.run_id));
    assert_eq!(
        store.manual_request(first_request).unwrap().result,
        "CANCELLED"
    );
    assert!(store.cancel_all_flow_runs().unwrap().is_empty());
}

#[test]
fn manual_runs_reject_frozen_locked_duplicate_and_capacity_executions() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    for flow_id in ["guarded-one", "guarded-two"] {
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
            .add_flow_task(task(directory.path(), flow_id, "root"))
            .unwrap();
        store.commit_flow(flow_id).unwrap();
    }
    store.freeze_flow("guarded-one").unwrap();
    assert!(matches!(
        store.create_flow_run("guarded-one", "MANUAL", false, None),
        Err(StoreError::InvalidData(message)) if message.contains("frozen")
    ));
    store.unfreeze_flow("guarded-one", None).unwrap();
    store.lock_queue().unwrap();
    assert!(matches!(
        store.create_flow_run("guarded-one", "MANUAL", false, None),
        Err(StoreError::QueueLocked)
    ));
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.set_scheduled_concurrency(1).unwrap();
    store.unlock_queue().unwrap();

    let first = store
        .create_flow_run("guarded-one", "MANUAL", false, None)
        .unwrap();
    assert!(matches!(
        store.create_flow_run("guarded-one", "MANUAL", false, None),
        Err(StoreError::InvalidData(message)) if message.contains("active run")
    ));
    let execution = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    assert_eq!(execution.run_id, first.run_id);
    assert!(matches!(
        store.create_flow_run("guarded-two", "MANUAL", false, None),
        Err(StoreError::InvalidData(message)) if message.contains("capacity")
    ));
}

#[test]
fn terminal_attempt_results_persist_cancellation_and_loss_details() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    flow(&store, directory.path());
    store.commit_flow("coverage-flow").unwrap();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();

    let cancelled = store
        .create_flow_run("coverage-flow", "MANUAL", false, None)
        .unwrap();
    let cancelled_attempt = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let cancelled_run = store
        .finish_flow_attempt(
            cancelled_attempt.attempt_id,
            FlowAttemptResult::Cancelled {
                detail: "cancelled by test".into(),
            },
        )
        .unwrap();
    assert_eq!(cancelled_run.run_id, cancelled.run_id);
    assert_eq!(cancelled_run.state, FlowRunState::Cancelled);
    let cancelled_history = store.list_flow_attempts(cancelled.run_id, "root").unwrap();
    assert_eq!(cancelled_history[0].state, AttemptState::Cancelled);
    assert_eq!(
        cancelled_history[0].failure_kind.as_deref(),
        Some("CANCELLED")
    );

    let lost = store
        .create_flow_run("coverage-flow", "MANUAL", false, None)
        .unwrap();
    let lost_attempt = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store
        .mark_flow_attempt_running(lost_attempt.attempt_id)
        .unwrap();
    let recovering = store
        .finish_flow_attempt(
            lost_attempt.attempt_id,
            FlowAttemptResult::Lost {
                detail: "worker disappeared".into(),
            },
        )
        .unwrap();
    assert_eq!(recovering.run_id, lost.run_id);
    assert_eq!(recovering.state, FlowRunState::Recovering);
    assert!(store.queue_locked().unwrap());
    assert!(store.queue_recovery_fence().unwrap());
    let lost_history = store.list_flow_attempts(lost.run_id, "root").unwrap();
    assert_eq!(lost_history[0].state, AttemptState::Lost);
    assert_eq!(lost_history[0].failure_kind.as_deref(), Some("LOST"));
}

#[test]
fn startup_failures_restore_manual_occurrences_and_skip_automatic_ones() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    flow(&store, directory.path());
    store.commit_flow("coverage-flow").unwrap();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();

    let manual = store
        .create_flow_run("coverage-flow", "MANUAL", true, None)
        .unwrap();
    let manual_attempt = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let failed_manual = store
        .finish_flow_attempt(
            manual_attempt.attempt_id,
            FlowAttemptResult::Failed {
                exit_code: None,
                kind: "SPAWN".into(),
                detail: "test startup failure".into(),
            },
        )
        .unwrap();
    assert_eq!(failed_manual.run_id, manual.run_id);
    assert_eq!(failed_manual.state, FlowRunState::FailedToStart);
    assert_eq!(
        store.list_occurrences("coverage-flow").unwrap()[0].state,
        OccurrenceState::Pending
    );

    let automatic_directory = tempfile::tempdir().unwrap();
    let automatic_store = Store::open(automatic_directory.path().join("stoker.db")).unwrap();
    automatic_store
        .create_flow(
            "automatic".into(),
            "Automatic".into(),
            "tester".into(),
            ScheduleSpec::Once {
                at: Utc::now() - Duration::seconds(1),
            },
        )
        .unwrap();
    automatic_store
        .add_flow_task(task(automatic_directory.path(), "automatic", "root"))
        .unwrap();
    automatic_store.commit_flow("automatic").unwrap();
    automatic_store.lock_queue().unwrap();
    automatic_store.set_mode(ExecutionMode::Scheduled).unwrap();
    automatic_store.unlock_queue().unwrap();
    let automatic_attempt = automatic_store
        .claim_flow_task(Utc::now())
        .unwrap()
        .unwrap();
    let failed_automatic = automatic_store
        .finish_flow_attempt(
            automatic_attempt.attempt_id,
            FlowAttemptResult::Failed {
                exit_code: None,
                kind: "STARTUP_TIMEOUT".into(),
                detail: "test timeout".into(),
            },
        )
        .unwrap();
    assert_eq!(failed_automatic.state, FlowRunState::FailedToStart);
    assert_eq!(
        automatic_store.list_occurrences("automatic").unwrap()[0].state,
        OccurrenceState::Skipped
    );
}
