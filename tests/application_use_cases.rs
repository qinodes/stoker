use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{FutureExt, StreamExt, stream};
use stoker::application::ports::{
    ConfigurationReader, ConfigurationRepositoryError, ConfigurationSnapshots, ConfigurationWriter,
    DescriptionUpdater, JobArtifacts, JobArtifactsError, JobCleaner, JobCreator, JobQueries,
    JobRepositoryError, LogEventStream, QueueRepository, SchedulerCancelGateway,
    SchedulerCommitGateway, SchedulerGatewayError, SchedulerLogGateway, SchedulerQueueGateway,
    SchedulerStatusGateway, WorkingDirectoryResolver,
};
use stoker::application::{
    self, ApplicationConfig, ApplicationError, ApplicationErrorCode, CommitSelection,
    ConfigSnapshot, Conflict, CreateJobInput, DescriptionUpdate, JobFilter, JobLogs, LogContent,
    LogEvent, Operation, OutputStream, PreparedJobInput, QueueMove, QueueSnapshot, SchedulerStatus,
    SnapshotReason,
};
use stoker::{Job, JobState};
use uuid::Uuid;

fn job(id: Uuid, state: JobState) -> Job {
    Job {
        id,
        name: "build".to_owned(),
        user: "alice".to_owned(),
        cwd: PathBuf::from("/work"),
        command: vec!["echo".to_owned(), "ok".to_owned()],
        command_line: Some("echo ok".to_owned()),
        state,
        queue_order: (state == JobState::Queued).then_some(1),
        created_at: Utc::now(),
        committed_at: None,
        started_at: None,
        finished_at: None,
        exit_code: None,
        pid: None,
        failure_detail: None,
        description: None,
        description_revision: 0,
    }
}

fn ready<T>(future: impl Future<Output = T>) -> T {
    future
        .now_or_never()
        .expect("application fake unexpectedly yielded")
}

#[derive(Default)]
struct FakeRepository {
    jobs: Mutex<Vec<Job>>,
    locked: Mutex<bool>,
    removed: Mutex<Vec<Uuid>>,
    logs: Mutex<(LogContent, LogContent)>,
}

impl FakeRepository {
    fn with_jobs(jobs: Vec<Job>) -> Self {
        Self {
            jobs: Mutex::new(jobs),
            ..Self::default()
        }
    }
}

impl JobQueries for FakeRepository {
    fn get_job(&self, id: Uuid) -> Result<Job, JobRepositoryError> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|job| job.id == id)
            .cloned()
            .ok_or(JobRepositoryError::NotFound { id })
    }

    fn list_jobs(&self, filter: &JobFilter) -> Result<Vec<Job>, JobRepositoryError> {
        Ok(self
            .jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|job| filter.user.as_ref().is_none_or(|user| &job.user == user))
            .filter(|job| filter.state.is_none_or(|state| job.state == state))
            .cloned()
            .collect())
    }
}

impl JobCreator for FakeRepository {
    fn create_job(&self, input: PreparedJobInput) -> Result<Job, JobRepositoryError> {
        let mut created = job(Uuid::nil(), JobState::Draft);
        created.user = input.user;
        created.name = input.name;
        created.description = input.description;
        created.cwd = input.cwd;
        created.command = input.command;
        created.command_line = Some(input.command_line);
        self.jobs.lock().unwrap().push(created.clone());
        Ok(created)
    }
}

impl DescriptionUpdater for FakeRepository {
    fn update_description(&self, input: DescriptionUpdate) -> Result<Job, JobRepositoryError> {
        let mut jobs = self.jobs.lock().unwrap();
        let job = jobs
            .iter_mut()
            .find(|job| job.id == input.id)
            .ok_or(JobRepositoryError::NotFound { id: input.id })?;
        if job.description_revision != input.expected_revision {
            return Err(JobRepositoryError::Conflict(Conflict::StaleDescription {
                id: input.id,
                expected_revision: input.expected_revision,
                actual_revision: job.description_revision,
            }));
        }
        job.description = input.description;
        job.description_revision += 1;
        Ok(job.clone())
    }
}

impl JobCleaner for FakeRepository {
    fn clean_terminal_jobs(&self) -> Result<Vec<Job>, JobRepositoryError> {
        let mut jobs = self.jobs.lock().unwrap();
        let mut removed = Vec::new();
        jobs.retain(|job| {
            let terminal = matches!(
                job.state,
                JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
            );
            if terminal {
                removed.push(job.clone());
            }
            !terminal
        });
        Ok(removed)
    }
}

impl JobArtifacts for FakeRepository {
    fn remove_job_artifacts(&self, id: Uuid) -> Result<(), JobArtifactsError> {
        self.removed.lock().unwrap().push(id);
        Ok(())
    }

    fn read_log(
        &self,
        _id: Uuid,
        stream: OutputStream,
        _max_bytes: Option<usize>,
    ) -> Result<LogContent, JobArtifactsError> {
        let logs = self.logs.lock().unwrap();
        Ok(match stream {
            OutputStream::Stdout => logs.0.clone(),
            OutputStream::Stderr => logs.1.clone(),
        })
    }
}

impl QueueRepository for FakeRepository {
    fn queue_snapshot(&self) -> Result<QueueSnapshot, JobRepositoryError> {
        Ok(QueueSnapshot {
            jobs: self
                .jobs
                .lock()
                .unwrap()
                .iter()
                .filter(|job| job.state == JobState::Queued)
                .cloned()
                .collect(),
            locked: *self.locked.lock().unwrap(),
        })
    }

    fn set_queue_locked(&self, locked: bool) -> Result<QueueSnapshot, JobRepositoryError> {
        *self.locked.lock().unwrap() = locked;
        self.queue_snapshot()
    }

    fn move_queued(&self, movement: QueueMove) -> Result<QueueSnapshot, JobRepositoryError> {
        let mut snapshot = self.queue_snapshot()?;
        if !snapshot.locked {
            return Err(JobRepositoryError::Conflict(Conflict::QueueUnlocked));
        }
        let Some(index) = snapshot.jobs.iter().position(|job| job.id == movement.id) else {
            return Err(JobRepositoryError::Conflict(Conflict::StaleQueue));
        };
        if !(1..=snapshot.jobs.len()).contains(&movement.target_order) {
            return Err(JobRepositoryError::Conflict(Conflict::StaleQueue));
        }
        let selected = snapshot.jobs.remove(index);
        snapshot.jobs.insert(movement.target_order - 1, selected);
        Ok(snapshot)
    }
}

struct FakeResolver(Result<PathBuf, String>);

impl WorkingDirectoryResolver for FakeResolver {
    fn resolve_working_directory(&self, _path: &Path) -> Result<PathBuf, String> {
        self.0.clone()
    }
}

#[derive(Clone, Copy)]
enum GatewayMode {
    Online,
    Rejected,
    OperationRejected,
    Unavailable,
    Timeout,
    OperationUnavailable,
    Stale,
}

struct FakeScheduler {
    mode: GatewayMode,
    job: Job,
}

impl FakeScheduler {
    fn failure(&self, operation: Operation) -> Option<SchedulerGatewayError> {
        match self.mode {
            GatewayMode::Rejected => {
                Some(SchedulerGatewayError::Rejected(Conflict::InvalidJobState {
                    id: self.job.id,
                    state: self.job.state,
                    operation: "test",
                }))
            }
            GatewayMode::Unavailable => Some(SchedulerGatewayError::Unavailable {
                message: "offline".to_owned(),
            }),
            GatewayMode::Timeout => Some(SchedulerGatewayError::Timeout { operation }),
            GatewayMode::Online
            | GatewayMode::OperationRejected
            | GatewayMode::OperationUnavailable
            | GatewayMode::Stale => None,
        }
    }

    fn operation_failure(&self, operation: Operation) -> Option<SchedulerGatewayError> {
        if matches!(self.mode, GatewayMode::OperationUnavailable) {
            Some(SchedulerGatewayError::Unavailable {
                message: "stopped during operation".to_owned(),
            })
        } else if matches!(self.mode, GatewayMode::OperationRejected) {
            Some(SchedulerGatewayError::Rejected(Conflict::QueueLocked))
        } else if matches!(self.mode, GatewayMode::Stale) {
            Some(SchedulerGatewayError::Rejected(Conflict::StaleQueue))
        } else {
            self.failure(operation)
        }
    }
}

#[async_trait]
impl SchedulerStatusGateway for FakeScheduler {
    async fn scheduler_status(&self) -> Result<SchedulerStatus, SchedulerGatewayError> {
        if let Some(error) = self.failure(Operation::Status) {
            return Err(error);
        }
        Ok(SchedulerStatus {
            pid: 7,
            active_job: None,
            queued_jobs: 1,
            queue_locked: true,
        })
    }
}

#[async_trait]
impl SchedulerQueueGateway for FakeScheduler {
    async fn set_queue_locked(&self, locked: bool) -> Result<QueueSnapshot, SchedulerGatewayError> {
        if let Some(error) = self.operation_failure(if locked {
            Operation::LockQueue
        } else {
            Operation::UnlockQueue
        }) {
            return Err(error);
        }
        Ok(QueueSnapshot {
            jobs: vec![self.job.clone()],
            locked,
        })
    }

    async fn move_queued(
        &self,
        _movement: QueueMove,
    ) -> Result<QueueSnapshot, SchedulerGatewayError> {
        if let Some(error) = self.operation_failure(Operation::MoveQueue) {
            return Err(error);
        }
        Ok(QueueSnapshot {
            jobs: vec![self.job.clone()],
            locked: true,
        })
    }
}

#[async_trait]
impl SchedulerCommitGateway for FakeScheduler {
    async fn commit(&self, _selection: CommitSelection) -> Result<Vec<Job>, SchedulerGatewayError> {
        if let Some(error) = self.operation_failure(Operation::Commit) {
            return Err(error);
        }
        Ok(vec![self.job.clone()])
    }
}

#[async_trait]
impl SchedulerCancelGateway for FakeScheduler {
    async fn cancel(&self, _id: Uuid) -> Result<Job, SchedulerGatewayError> {
        if let Some(error) = self.operation_failure(Operation::Cancel) {
            return Err(error);
        }
        Ok(self.job.clone())
    }
}

#[async_trait]
impl SchedulerLogGateway for FakeScheduler {
    async fn follow_logs(&self, _id: Uuid) -> Result<LogEventStream, SchedulerGatewayError> {
        if let Some(error) = self.operation_failure(Operation::FollowLogs) {
            return Err(error);
        }
        Ok(Box::pin(stream::iter([
            Ok(LogEvent::Chunk {
                stream: OutputStream::Stdout,
                bytes: b"hello".to_vec(),
            }),
            Ok(LogEvent::End),
        ])))
    }
}

#[test]
fn create_query_detail_description_and_clean_are_component_testable() {
    let id = Uuid::new_v4();
    let repository = FakeRepository::with_jobs(vec![job(id, JobState::Succeeded)]);
    let created = application::jobs::create_job(
        &repository,
        &FakeResolver(Ok(PathBuf::from("/resolved"))),
        CreateJobInput {
            user: "alice".to_owned(),
            name: "compile".to_owned(),
            description: Some("".to_owned()),
            cwd: PathBuf::from("input"),
            command_line: "echo 'hello world'".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(created.cwd, PathBuf::from("/resolved"));
    assert_eq!(created.command, ["echo", "hello world"]);
    assert_eq!(created.description, None);

    let listed = application::jobs::query_jobs(
        &repository,
        &JobFilter {
            user: Some("alice".to_owned()),
            state: Some(JobState::Draft),
        },
    )
    .unwrap();
    assert_eq!(listed, vec![created.clone()]);
    assert_eq!(
        application::jobs::job_detail(&repository, created.id).unwrap(),
        created
    );
    let updated = application::jobs::update_description(
        &repository,
        DescriptionUpdate {
            id: created.id,
            description: Some("documented".to_owned()),
            expected_revision: 0,
        },
    )
    .unwrap();
    assert_eq!(updated.description.as_deref(), Some("documented"));

    let cleaned = application::jobs::clean_jobs(&repository, &repository).unwrap();
    assert_eq!(cleaned.len(), 1);
    assert_eq!(*repository.removed.lock().unwrap(), vec![id]);
}

#[test]
fn create_rejects_typed_input_command_and_working_directory_failures() {
    let repository = FakeRepository::default();
    let base = CreateJobInput {
        user: "alice".to_owned(),
        name: "compile".to_owned(),
        description: None,
        cwd: PathBuf::from("input"),
        command_line: "echo ok".to_owned(),
    };
    let mut invalid = base.clone();
    invalid.name = " ".to_owned();
    assert_eq!(
        application::jobs::create_job(&repository, &FakeResolver(Ok("/ok".into())), invalid)
            .unwrap_err()
            .code(),
        ApplicationErrorCode::InvalidInput
    );
    let mut invalid = base.clone();
    invalid.command_line = "echo 'open".to_owned();
    assert!(matches!(
        application::jobs::create_job(&repository, &FakeResolver(Ok("/ok".into())), invalid),
        Err(ApplicationError::InvalidCommand { .. })
    ));
    assert!(matches!(
        application::jobs::create_job(&repository, &FakeResolver(Err("missing".to_owned())), base),
        Err(ApplicationError::InvalidWorkingDirectory { .. })
    ));
}

#[test]
fn scheduler_job_use_cases_keep_typed_success_rejection_unavailable_and_timeout() {
    let queued = job(Uuid::new_v4(), JobState::Queued);
    let scheduler = |mode| FakeScheduler {
        mode,
        job: queued.clone(),
    };
    assert_eq!(
        ready(application::jobs::commit_jobs(
            &scheduler(GatewayMode::Online),
            CommitSelection::All,
        ))
        .unwrap(),
        vec![queued.clone()]
    );
    assert_eq!(
        ready(application::jobs::cancel_job(
            &scheduler(GatewayMode::Online),
            queued.id,
        ))
        .unwrap(),
        queued
    );
    for (mode, code) in [
        (GatewayMode::Rejected, ApplicationErrorCode::Conflict),
        (GatewayMode::Unavailable, ApplicationErrorCode::Unavailable),
        (GatewayMode::Timeout, ApplicationErrorCode::Timeout),
    ] {
        assert_eq!(
            ready(application::jobs::commit_jobs(
                &scheduler(mode),
                CommitSelection::All,
            ))
            .unwrap_err()
            .code(),
            code
        );
    }
}

#[test]
fn queue_policy_falls_back_only_for_unavailable_and_preserves_stale_conflicts() {
    let queued = job(Uuid::new_v4(), JobState::Queued);
    let repository = FakeRepository::with_jobs(vec![queued.clone()]);
    *repository.locked.lock().unwrap() = true;
    let scheduler = |mode| FakeScheduler {
        mode,
        job: queued.clone(),
    };

    let online = ready(application::queue::queue_status(
        &repository,
        &scheduler(GatewayMode::Online),
    ))
    .unwrap();
    assert!(online.scheduler_online());
    let offline = ready(application::queue::queue_status(
        &repository,
        &scheduler(GatewayMode::Unavailable),
    ))
    .unwrap();
    assert!(!offline.scheduler_online());

    let fallback = ready(application::queue::set_queue_locked(
        &repository,
        &scheduler(GatewayMode::OperationUnavailable),
        false,
    ))
    .unwrap();
    assert!(!fallback.after.scheduler_online());
    assert!(!fallback.after.snapshot.locked);

    let timeout = ready(application::queue::queue_status(
        &repository,
        &scheduler(GatewayMode::Timeout),
    ))
    .unwrap_err();
    assert_eq!(timeout.code(), ApplicationErrorCode::Timeout);

    let stale = ready(application::queue::move_queued(
        &repository,
        &scheduler(GatewayMode::Stale),
        QueueMove {
            id: queued.id,
            target_order: 1,
        },
    ))
    .unwrap_err();
    assert!(matches!(
        stale,
        ApplicationError::Conflict(Conflict::StaleQueue)
    ));
}

#[test]
fn queue_online_mutations_and_mid_operation_fallback_cover_each_policy_branch() {
    let first = job(Uuid::new_v4(), JobState::Queued);
    let second = job(Uuid::new_v4(), JobState::Queued);
    let repository = FakeRepository::with_jobs(vec![first.clone(), second]);
    let scheduler = |mode| FakeScheduler {
        mode,
        job: first.clone(),
    };

    let locked = ready(application::queue::set_queue_locked(
        &repository,
        &scheduler(GatewayMode::Online),
        true,
    ))
    .unwrap();
    assert!(locked.after.scheduler_online());
    assert!(locked.after.snapshot.locked);
    // The production scheduler and repository share SQLite. Mirror that
    // externally observable state change in this isolated fake.
    *repository.locked.lock().unwrap() = true;

    let moved = ready(application::queue::move_queued(
        &repository,
        &scheduler(GatewayMode::Online),
        QueueMove {
            id: first.id,
            target_order: 1,
        },
    ))
    .unwrap();
    assert!(moved.scheduler_online());

    let fallback = ready(application::queue::move_queued(
        &repository,
        &scheduler(GatewayMode::OperationUnavailable),
        QueueMove {
            id: first.id,
            target_order: 2,
        },
    ))
    .unwrap();
    assert!(!fallback.scheduler_online());
    assert_eq!(fallback.snapshot.jobs[1].id, first.id);

    let rejected = ready(application::queue::set_queue_locked(
        &repository,
        &scheduler(GatewayMode::OperationRejected),
        false,
    ))
    .unwrap_err();
    assert_eq!(rejected.code(), ApplicationErrorCode::Conflict);

    let offline_move = ready(application::queue::move_queued(
        &repository,
        &scheduler(GatewayMode::Unavailable),
        QueueMove {
            id: first.id,
            target_order: 1,
        },
    ))
    .unwrap();
    assert!(!offline_move.scheduler_online());
}

#[test]
fn bounded_reads_and_live_streams_are_separate_log_use_cases() {
    let id = Uuid::new_v4();
    let repository = FakeRepository::with_jobs(vec![job(id, JobState::Running)]);
    *repository.logs.lock().unwrap() = (
        LogContent {
            bytes: b"tail".to_vec(),
            available: true,
            truncated: true,
            capture_error: None,
        },
        LogContent::default(),
    );
    let JobLogs {
        stdout, message, ..
    } = application::logs::read_logs(&repository, &repository, id, Some(4)).unwrap();
    assert_eq!(stdout.bytes, b"tail");
    assert!(stdout.truncated);
    assert_eq!(message, None);

    let scheduler = FakeScheduler {
        mode: GatewayMode::Online,
        job: job(id, JobState::Running),
    };
    let stream = ready(application::logs::follow_logs(&scheduler, id)).unwrap();
    let events = ready(stream.collect::<Vec<_>>());
    assert_eq!(events.len(), 2);
    assert!(matches!(events[1], Ok(LogEvent::End)));
}

#[derive(Default)]
struct FakeConfiguration {
    value: Mutex<ApplicationConfig>,
    snapshots: Mutex<Vec<ConfigSnapshot>>,
}

impl ConfigurationReader for FakeConfiguration {
    fn read_configuration(&self) -> Result<ApplicationConfig, ConfigurationRepositoryError> {
        Ok(self.value.lock().unwrap().clone())
    }
}

impl ConfigurationWriter for FakeConfiguration {
    fn write_configuration(
        &self,
        configuration: &ApplicationConfig,
    ) -> Result<(), ConfigurationRepositoryError> {
        *self.value.lock().unwrap() = configuration.clone();
        Ok(())
    }
}

impl ConfigurationSnapshots for FakeConfiguration {
    fn list_snapshots(&self) -> Result<Vec<ConfigSnapshot>, ConfigurationRepositoryError> {
        Ok(self.snapshots.lock().unwrap().clone())
    }

    fn create_snapshot(
        &self,
        reason: SnapshotReason,
    ) -> Result<ConfigSnapshot, ConfigurationRepositoryError> {
        let snapshot = ConfigSnapshot {
            path: PathBuf::from("snapshot.json"),
            valid: true,
            created_at: None,
            reason: Some(reason),
            timezone: self.value.lock().unwrap().timezone.clone(),
            error: None,
        };
        self.snapshots.lock().unwrap().push(snapshot.clone());
        Ok(snapshot)
    }

    fn restore_snapshot(
        &self,
        path: &Path,
    ) -> Result<ApplicationConfig, ConfigurationRepositoryError> {
        self.snapshots
            .lock()
            .unwrap()
            .iter()
            .find(|snapshot| snapshot.path == path)
            .map(|snapshot| ApplicationConfig {
                timezone: snapshot.timezone.clone(),
            })
            .ok_or_else(|| ConfigurationRepositoryError::SnapshotNotFound {
                path: path.to_path_buf(),
            })
    }
}

#[test]
fn configuration_timezone_and_snapshot_flows_need_no_filesystem() {
    let repository = FakeConfiguration::default();
    let configured =
        application::configuration::set_timezone(&repository, "Asia/Tokyo".to_owned()).unwrap();
    assert_eq!(configured.timezone.as_deref(), Some("Asia/Tokyo"));
    let snapshot =
        application::configuration::create_snapshot(&repository, SnapshotReason::Manual).unwrap();
    assert_eq!(
        application::configuration::list_snapshots(&repository).unwrap(),
        vec![snapshot.clone()]
    );
    assert_eq!(
        application::configuration::restore_snapshot(&repository, &snapshot.path).unwrap(),
        configured
    );
    assert_eq!(
        application::configuration::unset_timezone(&repository)
            .unwrap()
            .timezone,
        None
    );
    assert_eq!(
        application::configuration::set_timezone(&repository, "Mars/Base".to_owned())
            .unwrap_err()
            .code(),
        ApplicationErrorCode::InvalidInput
    );
    assert_eq!(
        application::configuration::set_timezone(&repository, "  ".to_owned())
            .unwrap_err()
            .code(),
        ApplicationErrorCode::InvalidInput
    );
    assert_eq!(
        application::configuration::restore_snapshot(&repository, Path::new("missing.json"))
            .unwrap_err()
            .code(),
        ApplicationErrorCode::NotFound
    );
}
