use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use stoker::domain::flow::{ExecutionMode, FlowRunState};
use stoker::scheduler::Scheduler;
use stoker::store::FlowTaskInput;
use stoker::{JobState, NewJob, StokerPaths, Store};
use tokio::sync::watch;

fn future_once() -> stoker::domain::flow::ScheduleSpec {
    stoker::domain::flow::ScheduleSpec::Once {
        at: Utc::now() + ChronoDuration::hours(2),
    }
}

fn use_scheduled_mode(store: &Store) {
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();
}

fn test_paths(root: &std::path::Path) -> StokerPaths {
    StokerPaths {
        root: root.to_path_buf(),
        database: root.join("stoker.db"),
        runs: root.join("runs"),
        lock: root.join("stoker.lock"),
        endpoint: root.join("stoker.sock"),
    }
}

#[tokio::test]
async fn scheduled_flow_manual_run_persists_attempt_logs() {
    let directory = tempfile::tempdir().unwrap();
    let paths = test_paths(directory.path());
    paths.ensure().unwrap();
    let store = Arc::new(Store::open(&paths.database).unwrap());
    let flow = store
        .create_flow(
            "flow-smoke".into(),
            "Smoke".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "first".into(),
            name: "First".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "echo flow-smoke".into(),
            retry: 0,
            dependencies: vec![],
            depend_mode: Default::default(),
        })
        .unwrap();
    store.commit_flow(&flow.flow_id).unwrap();
    use_scheduled_mode(&store);
    let run = store
        .create_flow_run(&flow.flow_id, "MANUAL", false, None)
        .unwrap();
    let scheduler = Arc::new(Scheduler::new(paths.clone(), Arc::clone(&store)));
    let (_wake_tx, wake_rx) = watch::channel(0_u64);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(Arc::clone(&scheduler).run(wake_rx, shutdown_rx));
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = store.get_flow_run(run.run_id).unwrap();
            if current.state.is_terminal() {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        eprintln!(
            "flow run did not finish: {:?}",
            store.get_flow_run(run.run_id)
        );
        panic!("flow run timed out");
    });
    assert_eq!(completed.state, FlowRunState::Succeeded);
    assert_eq!(completed.tasks[0].attempt_count, 1);
    let log = paths
        .runs
        .join("flows")
        .join(run.run_id.to_string())
        .join("first")
        .join("attempt-1")
        .join("stdout.log");
    assert!(std::fs::read_to_string(log).unwrap().contains("flow-smoke"));
    shutdown_tx.send(true).unwrap();
    task.await.unwrap().unwrap();
}

#[test]
fn flow_retries_are_durable_and_dependency_branches_are_isolated() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let flow = store
        .create_flow(
            "graph-flow".into(),
            "Graph".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "root".into(),
            name: "Root".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "echo root".into(),
            retry: 1,
            dependencies: vec![],
            depend_mode: Default::default(),
        })
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "after-success".into(),
            name: "After success".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "echo success".into(),
            retry: 0,
            dependencies: vec![stoker::domain::flow::Dependency {
                upstream_task_id: "root".into(),
                status: "succeeded".parse().unwrap(),
            }],
            depend_mode: Default::default(),
        })
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "after-failure".into(),
            name: "After failure".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "echo failure".into(),
            retry: 0,
            dependencies: vec![stoker::domain::flow::Dependency {
                upstream_task_id: "root".into(),
                status: "failed".parse().unwrap(),
            }],
            depend_mode: Default::default(),
        })
        .unwrap();
    store.commit_flow(&flow.flow_id).unwrap();
    use_scheduled_mode(&store);
    let run = store
        .create_flow_run(&flow.flow_id, "MANUAL", false, None)
        .unwrap();
    let first = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store.mark_flow_attempt_running(first.attempt_id).unwrap();
    let retry_run = store
        .finish_flow_attempt(
            first.attempt_id,
            stoker::store::FlowAttemptResult::Failed {
                exit_code: Some(2),
                kind: "EXIT".into(),
                detail: "first attempt".into(),
            },
        )
        .unwrap();
    assert_eq!(retry_run.state, FlowRunState::Running);
    assert_eq!(
        retry_run
            .tasks
            .iter()
            .find(|task| task.task_id == "root")
            .unwrap()
            .state,
        stoker::domain::flow::TaskRunState::RetryWait
    );
    let second = store
        .claim_flow_task(Utc::now() + ChronoDuration::seconds(6))
        .unwrap()
        .unwrap();
    let running_run = store
        .finish_flow_attempt(
            second.attempt_id,
            stoker::store::FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(running_run.state, FlowRunState::Running);
    let third = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let final_run = store
        .finish_flow_attempt(
            third.attempt_id,
            stoker::store::FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(final_run.state, FlowRunState::Succeeded);
    assert_eq!(
        final_run
            .tasks
            .iter()
            .find(|task| task.task_id == "root")
            .unwrap()
            .attempt_count,
        2
    );
    assert_eq!(
        final_run
            .tasks
            .iter()
            .find(|task| task.task_id == "after-success")
            .unwrap()
            .state,
        stoker::domain::flow::TaskRunState::Succeeded
    );
    assert_eq!(
        final_run
            .tasks
            .iter()
            .find(|task| task.task_id == "after-failure")
            .unwrap()
            .state,
        stoker::domain::flow::TaskRunState::Skipped
    );
    assert_eq!(run.flow_id, final_run.flow_id);
}

#[test]
fn invalid_flow_graph_is_rejected_at_commit() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let flow = store
        .create_flow(
            "cycle-flow".into(),
            "Cycle".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    let dependency = |upstream: &str| stoker::domain::flow::Dependency {
        upstream_task_id: upstream.into(),
        status: "succeeded".parse().unwrap(),
    };
    for (task_id, upstream) in [("a", "b"), ("b", "a")] {
        store
            .add_flow_task(FlowTaskInput {
                flow_id: flow.flow_id.clone(),
                task_id: task_id.into(),
                name: task_id.into(),
                cwd: directory.path().to_string_lossy().into_owned(),
                command: "echo cycle".into(),
                retry: 0,
                dependencies: vec![dependency(upstream)],
                depend_mode: Default::default(),
            })
            .unwrap();
    }
    assert!(store.commit_flow(&flow.flow_id).is_err());
}

#[test]
fn scheduled_once_occurrence_starts_automatically_within_its_window() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let due = Utc::now() - ChronoDuration::seconds(1);
    let flow = store
        .create_flow(
            "once-flow".into(),
            "Once".into(),
            "tester".into(),
            stoker::domain::flow::ScheduleSpec::Once { at: due },
        )
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "once".into(),
            name: "Once".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "echo once".into(),
            retry: 0,
            dependencies: vec![],
            depend_mode: Default::default(),
        })
        .unwrap();
    store.commit_flow(&flow.flow_id).unwrap();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();
    let task = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let run = store
        .finish_flow_attempt(
            task.attempt_id,
            stoker::store::FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(run.state, FlowRunState::Succeeded);
    assert_eq!(
        store.list_occurrences(&flow.flow_id).unwrap()[0].state,
        stoker::domain::flow::OccurrenceState::Started
    );
}

#[test]
fn scheduled_standalone_uses_a_stable_hidden_flow_and_updates_job_state() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let job_id = store
        .create_job(NewJob {
            name: "scheduled-job".into(),
            user: "tester".into(),
            description: None,
            cwd: directory.path().to_path_buf(),
            command: vec!["echo".into(), "scheduled".into()],
        })
        .unwrap();
    store
        .configure_standalone(
            job_id,
            ExecutionMode::Scheduled,
            Some(stoker::domain::flow::ScheduleSpec::Once {
                at: Utc::now() - ChronoDuration::seconds(1),
            }),
            0,
        )
        .unwrap();
    store.commit_job(job_id).unwrap();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();

    assert!(store.list_flows(None).unwrap().is_empty());
    let claimed = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    assert_eq!(store.get_job(job_id).unwrap().state, JobState::Starting);
    store.mark_flow_attempt_running(claimed.attempt_id).unwrap();
    let run = store
        .finish_flow_attempt(
            claimed.attempt_id,
            stoker::store::FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(run.flow_id, format!("standalone/{job_id}"));
    assert_eq!(run.state, FlowRunState::Succeeded);
    assert_eq!(store.get_job(job_id).unwrap().state, JobState::Succeeded);
    assert_eq!(
        store
            .list_flow_runs(&format!("standalone/{job_id}"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn serial_standalone_retry_is_durable_without_changing_legacy_job_identity() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let job_id = store
        .create_job(NewJob {
            name: "retry-job".into(),
            user: "tester".into(),
            description: None,
            cwd: directory.path().to_path_buf(),
            command: vec!["echo".into(), "retry".into()],
        })
        .unwrap();
    store
        .configure_standalone(job_id, ExecutionMode::Serial, None, 1)
        .unwrap();
    store.commit_job(job_id).unwrap();

    let first = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store.mark_flow_attempt_running(first.attempt_id).unwrap();
    let waiting = store
        .finish_flow_attempt(
            first.attempt_id,
            stoker::store::FlowAttemptResult::Failed {
                exit_code: Some(1),
                kind: "EXIT".into(),
                detail: "retry me".into(),
            },
        )
        .unwrap();
    assert_eq!(waiting.state, FlowRunState::Running);
    let second = store
        .claim_flow_task(Utc::now() + ChronoDuration::seconds(6))
        .unwrap()
        .unwrap();
    let finished = store
        .finish_flow_attempt(
            second.attempt_id,
            stoker::store::FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(finished.state, FlowRunState::Succeeded);
    assert_eq!(store.get_job(job_id).unwrap().state, JobState::Succeeded);
    assert_eq!(finished.tasks[0].attempt_count, 2);
}

#[test]
fn started_root_failure_runs_failed_branch_and_finishes_flow_failed() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("stoker.db")).unwrap();
    let flow = store
        .create_flow(
            "failure-flow".into(),
            "Failure".into(),
            "tester".into(),
            future_once(),
        )
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "root".into(),
            name: "Root".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "exit 1".into(),
            retry: 0,
            dependencies: vec![],
            depend_mode: Default::default(),
        })
        .unwrap();
    store
        .add_flow_task(FlowTaskInput {
            flow_id: flow.flow_id.clone(),
            task_id: "on-failure".into(),
            name: "On failure".into(),
            cwd: directory.path().to_string_lossy().into_owned(),
            command: "echo recovered".into(),
            retry: 0,
            dependencies: vec![stoker::domain::flow::Dependency {
                upstream_task_id: "root".into(),
                status: "failed".parse().unwrap(),
            }],
            depend_mode: Default::default(),
        })
        .unwrap();
    store.commit_flow(&flow.flow_id).unwrap();
    use_scheduled_mode(&store);
    let run = store
        .create_flow_run(&flow.flow_id, "MANUAL", false, None)
        .unwrap();
    let root = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let after_root = store
        .finish_flow_attempt(
            root.attempt_id,
            stoker::store::FlowAttemptResult::Failed {
                exit_code: Some(1),
                kind: "EXIT".into(),
                detail: "expected failure".into(),
            },
        )
        .unwrap();
    assert_eq!(after_root.state, FlowRunState::Running);
    let branch = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    let finished = store
        .finish_flow_attempt(
            branch.attempt_id,
            stoker::store::FlowAttemptResult::Succeeded { exit_code: 0 },
        )
        .unwrap();
    assert_eq!(finished.run_id, run.run_id);
    assert_eq!(finished.state, FlowRunState::Failed);
}
