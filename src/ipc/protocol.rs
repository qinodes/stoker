//! Versioned, wire-only IPC data transfer objects.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{Job, JobState};

pub const IPC_VERSION: u16 = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcRequest {
    Status,
    Stop,
    Commit { id: Uuid },
    CommitMany { ids: Vec<Uuid> },
    CommitAll,
    CommitUser { user: String },
    Cancel { id: Uuid },
    FollowLogs { id: Uuid },
    LockQueue,
    UnlockQueue,
    MoveQueued { id: Uuid, target_order: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpcResponse {
    Ack,
    Job { job: JobDto },
    Jobs { jobs: Vec<JobDto> },
    Queue { jobs: Vec<JobDto>, locked: bool },
    Status(ServiceStatus),
    LogChunk { stream: LogStream, bytes: Vec<u8> },
    LogEnd,
    Error(IpcError),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub pid: u32,
    pub active_job: Option<Uuid>,
    pub queued_jobs: usize,
    pub queue_locked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorCode {
    InvalidRequest,
    ProtocolVersion,
    NotFound,
    InvalidState,
    QueueLocked,
    QueueUnlocked,
    StaleQueue,
    ShuttingDown,
    Timeout,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<IpcErrorDetails>,
}

impl IpcError {
    pub fn new(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: IpcErrorDetails) -> Self {
        self.details = Some(details);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IpcErrorDetails {
    ProtocolVersion {
        expected: u16,
        actual: u16,
    },
    Job {
        id: Uuid,
        state: Option<JobStateDto>,
        operation: String,
    },
    Queue {
        id: Option<Uuid>,
        target_order: Option<usize>,
        queued_count: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStateDto {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobDto {
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

impl From<crate::scheduler::SchedulerStatus> for ServiceStatus {
    fn from(status: crate::scheduler::SchedulerStatus) -> Self {
        Self {
            pid: status.pid,
            active_job: status.active_job,
            queued_jobs: status.queued_jobs,
            queue_locked: status.queue_locked,
        }
    }
}

impl From<crate::scheduler::OutputStream> for LogStream {
    fn from(stream: crate::scheduler::OutputStream) -> Self {
        match stream {
            crate::scheduler::OutputStream::Stdout => Self::Stdout,
            crate::scheduler::OutputStream::Stderr => Self::Stderr,
        }
    }
}

impl From<JobState> for JobStateDto {
    fn from(state: JobState) -> Self {
        match state {
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

impl From<JobStateDto> for JobState {
    fn from(state: JobStateDto) -> Self {
        match state {
            JobStateDto::Draft => Self::Draft,
            JobStateDto::Queued => Self::Queued,
            JobStateDto::Starting => Self::Starting,
            JobStateDto::Running => Self::Running,
            JobStateDto::Cancelling => Self::Cancelling,
            JobStateDto::Succeeded => Self::Succeeded,
            JobStateDto::Failed => Self::Failed,
            JobStateDto::Cancelled => Self::Cancelled,
            JobStateDto::Lost => Self::Lost,
        }
    }
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

impl From<JobDto> for Job {
    fn from(job: JobDto) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_dto_round_trip_is_explicit_and_lossless() {
        let job = Job {
            id: Uuid::nil(),
            name: "build".to_owned(),
            user: "alice".to_owned(),
            cwd: PathBuf::from("workspace"),
            command: vec!["echo".to_owned()],
            command_line: Some("echo".to_owned()),
            state: JobState::Queued,
            queue_order: Some(1),
            created_at: Utc::now(),
            committed_at: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
            pid: None,
            failure_detail: None,
            description: Some("fixture".to_owned()),
            description_revision: 2,
        };
        assert_eq!(Job::from(JobDto::from(job.clone())), job);
    }

    #[test]
    fn typed_error_serialization_keeps_code_and_details_separate_from_message() {
        let error = IpcError::new(IpcErrorCode::InvalidState, "cannot cancel").with_details(
            IpcErrorDetails::Job {
                id: Uuid::nil(),
                state: Some(JobStateDto::Succeeded),
                operation: "cancel".to_owned(),
            },
        );
        let json = serde_json::to_value(error).unwrap();
        assert_eq!(json["code"], "invalid_state");
        assert_eq!(json["details"]["kind"], "job");
        assert_eq!(json["details"]["state"], "SUCCEEDED");
    }

    #[test]
    fn every_job_state_maps_both_directions() {
        for state in [
            JobState::Draft,
            JobState::Queued,
            JobState::Starting,
            JobState::Running,
            JobState::Cancelling,
            JobState::Succeeded,
            JobState::Failed,
            JobState::Cancelled,
            JobState::Lost,
        ] {
            assert_eq!(JobState::from(JobStateDto::from(state)), state);
        }
    }
}
