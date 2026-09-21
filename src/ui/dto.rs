use std::net::IpAddr;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::application::ConfigSnapshot;
use crate::config::{
    LogPolicy, POLICY_MB_BYTES, ResolvedTimezone, RuntimePolicy, StokerConfig, TimezoneSource,
};
use crate::domain::{Job, JobState};

pub(in crate::ui) mod scheduled;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiMetadata {
    pub pid: u32,
    pub host: IpAddr,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub(super) enum JobStateDto {
    Draft,
    Queued,
    Starting,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Lost,
}

impl From<JobState> for JobStateDto {
    fn from(value: JobState) -> Self {
        match value {
            JobState::Draft => Self::Draft,
            JobState::Queued => Self::Queued,
            JobState::Starting => Self::Starting,
            JobState::Running => Self::Running,
            JobState::Cancelling => Self::Cancelling,
            JobState::Succeeded => Self::Succeeded,
            JobState::Failed => Self::Failed,
            JobState::Cancelled => Self::Cancelled,
            JobState::Lost => Self::Lost,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct JobDto {
    pub id: Uuid,
    pub name: String,
    pub user: String,
    pub cwd: PathBuf,
    pub command: Vec<String>,
    pub command_line: Option<String>,
    pub state: JobStateDto,
    pub queue_order: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub committed_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub pid: Option<u32>,
    pub failure_detail: Option<String>,
    pub description: Option<String>,
    pub description_revision: i64,
}

impl From<Job> for JobDto {
    fn from(job: Job) -> Self {
        Self {
            id: job.id,
            name: job.name,
            user: job.user,
            cwd: job.cwd,
            command: job.command,
            command_line: job.command_line,
            state: job.state.into(),
            queue_order: job.queue_order,
            created_at: job.created_at,
            committed_at: job.committed_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            exit_code: job.exit_code,
            pid: job.pid,
            failure_detail: job.failure_detail,
            description: job.description,
            description_revision: job.description_revision,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct UiConfigResponse {
    pub version: &'static str,
    pub max_job_name_length: usize,
    pub max_job_user_length: usize,
    pub max_job_description_length: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct StatusResponse {
    pub scheduler: SchedulerResponse,
    pub counts: CountResponse,
    pub queue_locked: bool,
    pub disk_pressure: bool,
    pub timezone: TimezoneResponse,
    pub generated_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkspaceResponse {
    pub mode: &'static str,
    pub queue_locked: bool,
    pub recovery_fence: bool,
    pub scheduler: WorkspaceSchedulerResponse,
    pub timezone: TimezoneResponse,
    pub generated_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkspaceSchedulerResponse {
    pub running: bool,
    pub pid: Option<u32>,
    pub active_job: Option<Uuid>,
    pub queued_jobs: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct SchedulerResponse {
    pub running: bool,
    pub pid: Option<u32>,
    pub active_job: Option<Uuid>,
    pub queued_jobs: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct CountResponse {
    pub total: usize,
    pub draft: usize,
    pub queued: usize,
    pub active: usize,
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct TimezoneResponse {
    pub name: String,
    pub source: &'static str,
}

impl From<&ResolvedTimezone> for TimezoneResponse {
    fn from(timezone: &ResolvedTimezone) -> Self {
        Self::new(timezone.name.clone(), timezone.source)
    }
}

impl TimezoneResponse {
    fn new(name: String, source: TimezoneSource) -> Self {
        Self {
            name,
            source: match source {
                TimezoneSource::Cli => "cli",
                TimezoneSource::Config => "config",
                TimezoneSource::System => "system",
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct JobsResponse {
    pub jobs: Vec<JobDto>,
    pub timezone: TimezoneResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct QueueResponse {
    pub jobs: Vec<JobDto>,
    pub locked: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct CreateJobResponse {
    pub job: JobDto,
}

#[derive(Debug, Serialize)]
pub(super) struct JobDetailResponse {
    pub job: JobDto,
    pub working_directory_status: &'static str,
    pub display_timezone: String,
}

#[derive(Debug, Serialize)]
pub(super) struct JobActionResponse {
    pub job: JobDto,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateJobRequest {
    pub user: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub cwd: String,
    pub command: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateDescriptionRequest {
    pub description: Option<String>,
    pub expected_revision: i64,
}

#[derive(Debug, Serialize)]
pub(super) struct FsLocation {
    pub kind: &'static str,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct FsRootsResponse {
    pub default_path: Option<String>,
    pub locations: Vec<FsLocation>,
}

#[derive(Debug, Serialize)]
pub(super) struct FsDirectory {
    pub name: String,
    pub path: String,
    pub is_symlink: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct FsDirectoriesResponse {
    pub path: String,
    pub parent: Option<String>,
    pub directories: Vec<FsDirectory>,
    pub truncated: bool,
    pub skipped_entries: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct CleanResponse {
    pub removed: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct LogsResponse {
    pub job: JobDto,
    pub stdout: String,
    pub stderr: String,
    pub stdout_available: bool,
    pub stderr_available: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_capture_error: Option<String>,
    pub stderr_capture_error: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ConfigurationResponse {
    pub config: StokerConfig,
    pub effective_timezone: TimezoneResponse,
    pub timezones: Vec<String>,
    pub config_path: String,
    pub snapshot_dir: String,
    pub snapshots: Vec<SnapshotResponse>,
}

/// The policy endpoint exposes the same whole-number units accepted by the
/// CLI. Log capacities are MB; runtime values are milliseconds.
#[derive(Debug, Serialize)]
pub(super) struct PolicyResponse {
    pub log: LogPolicyResponse,
    pub runtime: RuntimePolicyResponse,
    pub defaults: PolicyDefaultsResponse,
    pub units: PolicyUnitsResponse,
    pub queue_locked: bool,
    pub can_update: bool,
    pub active_jobs: Vec<PolicyActiveJobResponse>,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct LogPolicyResponse {
    pub max_bytes_per_job: u64,
    pub segment_bytes: u64,
    pub max_bytes_total: u64,
    pub retention_jobs: u64,
    pub disk_reserve_bytes: u64,
}

impl From<LogPolicy> for LogPolicyResponse {
    fn from(policy: LogPolicy) -> Self {
        Self {
            max_bytes_per_job: policy.max_bytes_per_job.div_ceil(POLICY_MB_BYTES),
            segment_bytes: policy.segment_bytes.div_ceil(POLICY_MB_BYTES),
            max_bytes_total: policy.max_bytes_total.div_ceil(POLICY_MB_BYTES),
            retention_jobs: policy.retention_jobs,
            disk_reserve_bytes: policy.disk_reserve_bytes.div_ceil(POLICY_MB_BYTES),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct RuntimePolicyResponse {
    pub termination_grace_ms: u64,
    pub max_runtime_ms: Option<u64>,
    pub startup_timeout_ms: u64,
}

impl From<RuntimePolicy> for RuntimePolicyResponse {
    fn from(policy: RuntimePolicy) -> Self {
        Self {
            termination_grace_ms: policy.termination_grace_ms,
            max_runtime_ms: policy.max_runtime_ms,
            startup_timeout_ms: policy.startup_timeout_ms,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct PolicyDefaultsResponse {
    pub log: LogPolicyResponse,
    pub runtime: RuntimePolicyResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct PolicyUnitsResponse {
    pub log: LogPolicyUnitsResponse,
    pub runtime: RuntimePolicyUnitsResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct LogPolicyUnitsResponse {
    pub max_bytes_per_job: &'static str,
    pub segment_bytes: &'static str,
    pub max_bytes_total: &'static str,
    pub retention_jobs: &'static str,
    pub disk_reserve_bytes: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct RuntimePolicyUnitsResponse {
    pub termination_grace_ms: &'static str,
    pub max_runtime_ms: &'static str,
    pub startup_timeout_ms: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct PolicyActiveJobResponse {
    pub id: Uuid,
    pub name: String,
    pub state: JobStateDto,
}

#[derive(Debug, Deserialize)]
pub(super) struct PolicyValueRequest {
    pub value: Value,
}

#[derive(Debug, Serialize)]
pub(super) struct SnapshotResponse {
    pub path: String,
    pub valid: bool,
    pub created_at: Option<String>,
    pub reason: Option<String>,
    pub timezone: Option<String>,
    pub error: Option<String>,
}

impl From<ConfigSnapshot> for SnapshotResponse {
    fn from(snapshot: ConfigSnapshot) -> Self {
        Self {
            path: snapshot.path.to_string_lossy().into_owned(),
            valid: snapshot.valid,
            created_at: snapshot.created_at.map(|value| value.to_rfc3339()),
            reason: snapshot.reason.map(|value| value.to_string()),
            timezone: snapshot.timezone,
            error: snapshot.error,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct QueueMoveRequest {
    pub target_order: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct TimezoneRequest {
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RestoreRequest {
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::SnapshotReason;

    #[test]
    fn job_mapping_is_a_wire_boundary_and_preserves_v1_fields() {
        let now = Utc::now();
        let job = Job {
            id: Uuid::nil(),
            name: "build".into(),
            user: "alice".into(),
            cwd: PathBuf::from("workspace"),
            command: vec!["cargo".into(), "test".into()],
            command_line: Some("cargo test".into()),
            state: JobState::Running,
            queue_order: Some(2),
            created_at: now,
            committed_at: Some(now),
            started_at: Some(now),
            finished_at: None,
            exit_code: None,
            pid: Some(42),
            failure_detail: None,
            description: Some("tests".into()),
            description_revision: 3,
        };
        let dto = JobDto::from(job.clone());
        let value = serde_json::to_value(dto).unwrap();
        let domain = serde_json::to_value(job).unwrap();
        assert_eq!(value["state"], "RUNNING");
        assert_eq!(value["command_line"], "cargo test");
        assert_eq!(value["description_revision"], 3);
        assert_eq!(
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            domain.as_object().unwrap().keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn every_job_state_has_an_explicit_wire_value() {
        let cases = [
            (JobState::Draft, "DRAFT"),
            (JobState::Queued, "QUEUED"),
            (JobState::Starting, "STARTING"),
            (JobState::Running, "RUNNING"),
            (JobState::Cancelling, "CANCELLING"),
            (JobState::Succeeded, "SUCCEEDED"),
            (JobState::Failed, "FAILED"),
            (JobState::Cancelled, "CANCELLED"),
            (JobState::Lost, "LOST"),
        ];
        for (state, expected) in cases {
            assert_eq!(
                serde_json::to_value(JobStateDto::from(state)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn timezone_and_snapshot_mappings_preserve_adapter_fields() {
        for (source, expected) in [
            (TimezoneSource::Cli, "cli"),
            (TimezoneSource::Config, "config"),
            (TimezoneSource::System, "system"),
        ] {
            assert_eq!(TimezoneResponse::new("UTC".into(), source).source, expected);
        }

        let created_at = Utc::now();
        let snapshot = SnapshotResponse::from(ConfigSnapshot {
            path: PathBuf::from("snapshot.toml"),
            valid: true,
            created_at: Some(created_at),
            reason: Some(SnapshotReason::Manual),
            timezone: Some("UTC".into()),
            error: None,
        });
        assert_eq!(snapshot.path, "snapshot.toml");
        assert_eq!(snapshot.created_at, Some(created_at.to_rfc3339()));
        assert_eq!(snapshot.reason.as_deref(), Some("manual"));
        assert_eq!(snapshot.timezone.as_deref(), Some("UTC"));
    }
}
