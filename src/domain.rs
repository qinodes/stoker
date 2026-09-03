use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum JobState {
    Draft,
    Queued,
    Paused,
    Starting,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Lost,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Queued => "QUEUED",
            Self::Paused => "PAUSED",
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Cancelling => "CANCELLING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::Lost => "LOST",
        }
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for JobState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "DRAFT" => Ok(Self::Draft),
            "QUEUED" => Ok(Self::Queued),
            "PAUSED" => Ok(Self::Paused),
            "STARTING" => Ok(Self::Starting),
            "RUNNING" => Ok(Self::Running),
            "CANCELLING" => Ok(Self::Cancelling),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "LOST" => Ok(Self::Lost),
            _ => Err(format!("unknown job state {value:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub name: String,
    pub user: String,
    pub cwd: PathBuf,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub name: String,
    pub user: String,
    pub cwd: PathBuf,
    pub command: Vec<String>,
    /// The original shell command for jobs added through the CLI.
    /// `None` identifies jobs created by older Stoker versions.
    pub command_line: Option<String>,
    pub state: JobState,
    pub queue_order: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub committed_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub pid: Option<u32>,
    pub failure_detail: Option<String>,
}
