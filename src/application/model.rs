use std::path::PathBuf;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::{Job, JobState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateJobInput {
    pub user: String,
    pub name: String,
    pub description: Option<String>,
    pub cwd: PathBuf,
    pub command_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedJobInput {
    pub user: String,
    pub name: String,
    pub description: Option<String>,
    pub cwd: PathBuf,
    pub command: Vec<String>,
    pub command_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobFilter {
    pub user: Option<String>,
    pub state: Option<JobState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptionUpdate {
    pub id: Uuid,
    pub description: Option<String>,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitSelection {
    Jobs(Vec<Uuid>),
    All,
    User(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueMove {
    pub id: Uuid,
    pub target_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub jobs: Vec<Job>,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueStatus {
    pub snapshot: QueueSnapshot,
    pub scheduler: Option<SchedulerStatus>,
}

impl QueueStatus {
    pub const fn scheduler_online(&self) -> bool {
        self.scheduler.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueLockResult {
    pub before: QueueStatus,
    pub after: QueueStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplicationConfig {
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotReason {
    Initial,
    BeforeConfigUpdate,
    BeforeRestore,
    Manual,
}

impl std::fmt::Display for SnapshotReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Initial => "initial",
            Self::BeforeConfigUpdate => "before config update",
            Self::BeforeRestore => "before restore",
            Self::Manual => "manual",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSnapshot {
    pub path: PathBuf,
    pub valid: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub reason: Option<SnapshotReason>,
    pub timezone: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerStatus {
    pub pid: u32,
    pub active_job: Option<Uuid>,
    pub queued_jobs: usize,
    pub queue_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    Chunk {
        stream: OutputStream,
        bytes: Vec<u8>,
    },
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogContent {
    pub bytes: Vec<u8>,
    pub available: bool,
    pub truncated: bool,
    pub capture_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLogs {
    pub job: Job,
    pub stdout: LogContent,
    pub stderr: LogContent,
    pub message: Option<String>,
}
