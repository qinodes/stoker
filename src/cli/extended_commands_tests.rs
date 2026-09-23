use super::super::extended_args::ExtendedCli;
use super::*;
use crate::domain::flow::{SchedulePeriod, SchedulePeriodUnit, ScheduleSpec};
use crate::{NewJob, config::StokerPaths};
use chrono::Utc;
use clap::Parser;

fn dispatch_args(paths: &StokerPaths, store: &Store, args: Vec<String>) -> Result<()> {
    let cli = ExtendedCli::try_parse_from(args)?;
    dispatch(paths, store, cli.command)
}

#[test]
fn standalone_command_handlers_cover_list_run_edit_and_control_paths() {
    let directory = tempfile::tempdir().unwrap();
    let paths = StokerPaths {
        root: directory.path().to_path_buf(),
        database: directory.path().join("stoker.db"),
        runs: directory.path().join("runs"),
        lock: directory.path().join("stoker.lock"),
        endpoint: directory.path().join("stoker.sock"),
    };
    paths.ensure().unwrap();
    let store = Store::open(&paths.database).unwrap();
    store.lock_queue().unwrap();
    store.set_mode(ExecutionMode::Scheduled).unwrap();
    store.unlock_queue().unwrap();

    let job_id = store
        .create_job(NewJob {
            name: "nightly".into(),
            user: "ops".into(),
            description: None,
            cwd: directory.path().to_path_buf(),
            command: vec!["echo".into(), "nightly".into()],
        })
        .unwrap();
    store
        .configure_standalone(
            job_id,
            ExecutionMode::Scheduled,
            Some(ScheduleSpec::Periodic {
                every: SchedulePeriod {
                    value: 15,
                    unit: SchedulePeriodUnit::Minutes,
                },
                first_at: None,
            }),
            1,
        )
        .unwrap();
    let flow_id = standalone_flow_id(job_id);
    store.commit_flow(&flow_id).unwrap();
    let id = job_id.to_string();

    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "mode".into(), "show".into()],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "jobs".into(),
            "--mode".into(),
            "scheduled".into(),
        ],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "show".into(), id.clone()],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "occurrences".into(), id.clone()],
    )
    .unwrap();

    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "disable".into(), id.clone()],
    )
    .unwrap();
    assert!(!store.standalone_definition(job_id).unwrap().enabled);
    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "enable".into(), id.clone()],
    )
    .unwrap();
    assert!(store.standalone_definition(job_id).unwrap().enabled);

    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "freeze".into(), id.clone()],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "schedule".into(),
            "set".into(),
            id.clone(),
            "--every".into(),
            "30m".into(),
            "--expected-draft-revision".into(),
            "0".into(),
        ],
    )
    .unwrap();
    assert!(store.get_flow(&flow_id).unwrap().has_draft);
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "draft".into(),
            "discard".into(),
            id.clone(),
            "--expected-draft-revision".into(),
            "1".into(),
        ],
    )
    .unwrap();
    assert!(!store.get_flow(&flow_id).unwrap().has_draft);
    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "unfreeze".into(), id.clone()],
    )
    .unwrap();
    assert!(!store.get_flow(&flow_id).unwrap().frozen);

    let request_id = Uuid::new_v4();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "run".into(),
            id.clone(),
            "--request-id".into(),
            request_id.to_string(),
        ],
    )
    .unwrap();
    let request = store.manual_request(request_id).unwrap();
    let run_id = request.run_id.unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "show".into(),
            id.clone(),
            "--run".into(),
            run_id.to_string(),
        ],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "request".into(),
            "show".into(),
            request_id.to_string(),
        ],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "runs".into(), id.clone()],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "logs".into(),
            id.clone(),
            "--run".into(),
            run_id.to_string(),
        ],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "cancel".into(),
            id.clone(),
            "--run".into(),
            run_id.to_string(),
        ],
    )
    .unwrap();

    store.lock_queue().unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "mode".into(),
            "set".into(),
            "scheduled".into(),
        ],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "policy".into(), "show".into()],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "policy".into(),
            "set".into(),
            "max-concurrency".into(),
            "3".into(),
        ],
    )
    .unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "policy".into(),
            "unset".into(),
            "max-concurrency".into(),
        ],
    )
    .unwrap();
    store.unlock_queue().unwrap();

    let recovery_request = Uuid::new_v4();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "run".into(),
            id.clone(),
            "--request-id".into(),
            recovery_request.to_string(),
        ],
    )
    .unwrap();
    let recovery_run = store
        .manual_request(recovery_request)
        .unwrap()
        .run_id
        .unwrap();
    let attempt = store.claim_flow_task(Utc::now()).unwrap().unwrap();
    store.mark_flow_attempt_running(attempt.attempt_id).unwrap();
    store.recover_runtime_jobs().unwrap();
    dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "recovery".into(),
            "reconcile".into(),
            recovery_run.to_string(),
            "--confirm-stopped".into(),
        ],
    )
    .unwrap();
    assert!(
        store
            .get_flow_run(recovery_run)
            .unwrap()
            .state
            .is_terminal()
    );

    let new_job = dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "create".into(),
            "--user".into(),
            "ops".into(),
            "--name".into(),
            "hourly".into(),
            "--cmd".into(),
            "echo hourly".into(),
            "--every".into(),
            "1h".into(),
            "--retry".into(),
            "2".into(),
        ],
    );
    new_job.unwrap();
    assert_eq!(
        store
            .list_jobs_for_mode(None, None, Some(ExecutionMode::Scheduled))
            .unwrap()
            .len(),
        2
    );

    let error = dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "policy".into(),
            "set".into(),
            "unknown".into(),
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("unknown extended policy"));
    let missing_job = dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "run".into(), Uuid::new_v4().to_string()],
    )
    .unwrap_err();
    assert!(!missing_job.to_string().is_empty());

    let invalid_jobs_mode = dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "jobs".into(),
            "--mode".into(),
            "unknown".into(),
        ],
    )
    .unwrap_err();
    assert!(!invalid_jobs_mode.to_string().is_empty());

    let invalid_timezone_option = dispatch_args(
        &paths,
        &store,
        vec![
            "stoker".into(),
            "create".into(),
            "--user".into(),
            "ops".into(),
            "--name".into(),
            "invalid".into(),
            "--cmd".into(),
            "echo invalid".into(),
            "--schedule-timezone".into(),
            "UTC".into(),
        ],
    )
    .unwrap_err();
    assert!(!invalid_timezone_option.to_string().is_empty());

    let missing_draft_revision = dispatch_args(
        &paths,
        &store,
        vec!["stoker".into(), "draft".into(), "discard".into(), id],
    )
    .unwrap_err();
    assert!(
        missing_draft_revision
            .to_string()
            .contains("--expected-draft-revision is required")
    );
}
