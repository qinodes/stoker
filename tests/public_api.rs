use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use stoker::application::ports::{
    Clock, ConfigurationReader, ConfigurationSnapshots, ConfigurationWriter, DescriptionUpdater,
    JobArtifacts, JobCanceller, JobCleaner, JobCommitter, JobCreator, JobQueries, QueueRepository,
    SchedulerCancelGateway, SchedulerCommitGateway, SchedulerLogGateway, SchedulerQueueGateway,
    SchedulerStatusGateway,
};
use stoker::application::{
    ApplicationErrorCode, CommitSelection, CreateJobInput, DescriptionUpdate, JobFilter, QueueMove,
};
use stoker::cli::{Cli, CliCommand};
use stoker::config::{ConfigSnapshotReason, StokerConfig, TimezoneSource};
use stoker::ipc::LogStream;
use stoker::process::{DefaultProcessController, ProcessSpec, SystemProcessController};
use stoker::scheduler::Scheduler;
use stoker::service::Service;
use stoker::store::CURRENT_SCHEMA_VERSION;
use stoker::ui::UiMetadata;
use stoker::{
    DomainErrorCode, IPC_VERSION, IpcError, IpcErrorCode, IpcRequest, IpcResponse, JobState,
    NewJob, ServiceClient, ServiceStatus, StokerPaths, Store, ValidationField,
};

#[test]
fn documented_public_facades_compile_for_an_external_consumer() {
    let directory = tempfile::tempdir().unwrap();
    let paths = StokerPaths {
        root: directory.path().to_path_buf(),
        database: directory.path().join("stoker.db"),
        runs: directory.path().join("runs"),
        lock: directory.path().join("stoker.lock"),
        endpoint: directory.path().join("stoker.sock"),
    };
    paths.ensure().unwrap();

    let cli = Cli::try_parse_from(["stoker", "status"]).unwrap();
    assert!(matches!(cli.command, CliCommand::Status));

    let store = Arc::new(Store::open(&paths.database).unwrap());
    let id = store
        .create_job(NewJob {
            name: "public-api".to_owned(),
            user: "fixture".to_owned(),
            description: None,
            cwd: directory.path().to_path_buf(),
            command: vec!["echo".to_owned(), "fixture".to_owned()],
        })
        .unwrap();
    assert_eq!(store.get_job(id).unwrap().state, JobState::Draft);
    assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert_eq!(store.clone().get_job(id).unwrap().id, id);

    let scheduler = Scheduler::new(paths.clone(), Arc::clone(&store));
    assert_eq!(scheduler.scheduler_status().unwrap().queued_jobs, 0);
    let client = ServiceClient::new(paths.clone());
    #[allow(deprecated)]
    let legacy_status: ServiceStatus = scheduler.service_status().unwrap();
    assert_eq!(legacy_status.queued_jobs, 0);
    #[allow(deprecated)]
    let legacy_log_future = client.follow_logs(id);
    fn accepts_legacy_log_result(_: impl std::future::Future<Output = anyhow::Result<()>>) {}
    accepts_legacy_log_result(legacy_log_future);
    let _service_constructor: fn(StokerPaths) -> anyhow::Result<Service> = Service::new;
    let _controller: SystemProcessController = DefaultProcessController::new();
    let _process = ProcessSpec {
        program: OsString::from("fixture"),
        args: Vec::new(),
        cwd: PathBuf::from("."),
        stdout_log: directory.path().join("stdout.log"),
        stderr_log: directory.path().join("stderr.log"),
    };

    assert_eq!(IPC_VERSION, 4);
    let status = ServiceStatus {
        pid: 7,
        active_job: None,
        queued_jobs: 0,
        queue_locked: false,
    };
    assert!(matches!(IpcRequest::Status, IpcRequest::Status));
    assert!(matches!(
        IpcResponse::LogChunk {
            stream: LogStream::Stdout,
            bytes: Vec::new(),
        },
        IpcResponse::LogChunk { .. }
    ));
    assert!(matches!(
        IpcResponse::Status(status),
        IpcResponse::Status(_)
    ));
    assert!(matches!(
        IpcResponse::Error(IpcError::new(IpcErrorCode::NotFound, "missing")),
        IpcResponse::Error(IpcError {
            code: IpcErrorCode::NotFound,
            ..
        })
    ));

    let metadata = UiMetadata {
        pid: 7,
        host: "127.0.0.1".parse().unwrap(),
        port: stoker::ui::default_port(),
    };
    assert_eq!(metadata.port, 8765);
    let _config = StokerConfig::default();
    let _snapshot_reason = ConfigSnapshotReason::Manual;
    let _timezone_source = TimezoneSource::System;

    assert!(stoker::validate_job_name("name").is_ok());
    assert!(stoker::validate_job_user("owner").is_ok());
    assert!(stoker::validate_description(Some("description")).is_ok());
    assert_eq!(stoker::normalize_description(Some(String::new())), None);

    let typed_error = stoker::domain::validation::validate_job_name(" ").unwrap_err();
    assert_eq!(typed_error.code(), DomainErrorCode::EmptyValue);
    assert!(matches!(
        typed_error,
        stoker::DomainError::EmptyValue {
            field: ValidationField::JobName
        }
    ));

    let _create = CreateJobInput {
        user: "fixture".to_owned(),
        name: "public-api".to_owned(),
        description: None,
        cwd: directory.path().to_path_buf(),
        command_line: "echo fixture".to_owned(),
    };
    let _filter = JobFilter::default();
    let _description = DescriptionUpdate {
        id,
        description: Some("new".to_owned()),
        expected_revision: 0,
    };
    let _selection = CommitSelection::Jobs(vec![id]);
    let _movement = QueueMove {
        id,
        target_order: 1,
    };
    assert_ne!(
        ApplicationErrorCode::InvalidInput,
        ApplicationErrorCode::Conflict
    );

    let _: Option<&dyn Clock> = None;
    let _: Option<&dyn JobQueries> = None;
    let _: Option<&dyn JobCreator> = None;
    let _: Option<&dyn DescriptionUpdater> = None;
    let _: Option<&dyn JobCleaner> = None;
    let _: Option<&dyn JobArtifacts> = None;
    let _: Option<&dyn JobCommitter> = None;
    let _: Option<&dyn JobCanceller> = None;
    let _: Option<&dyn QueueRepository> = None;
    let _: Option<&dyn ConfigurationReader> = None;
    let _: Option<&dyn ConfigurationWriter> = None;
    let _: Option<&dyn ConfigurationSnapshots> = None;
    let _: Option<&dyn SchedulerStatusGateway> = None;
    let _: Option<&dyn SchedulerCommitGateway> = None;
    let _: Option<&dyn SchedulerCancelGateway> = None;
    let _: Option<&dyn SchedulerQueueGateway> = None;
    let _: Option<&dyn SchedulerLogGateway> = None;
}
