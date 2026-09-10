use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum JobState {
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

impl JobState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Queued => "QUEUED",
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
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for JobState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        super::validation::parse_job_state(value).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub name: String,
    pub user: String,
    pub description: Option<String>,
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
    pub description: Option<String>,
    pub description_revision: i64,
}

#[cfg(test)]
mod tests {
    use super::JobState;

    #[test]
    fn job_states_round_trip_through_display_and_compatibility_parser() {
        let states = [
            JobState::Draft,
            JobState::Queued,
            JobState::Starting,
            JobState::Running,
            JobState::Cancelling,
            JobState::Succeeded,
            JobState::Failed,
            JobState::Cancelled,
            JobState::Lost,
        ];

        for state in states {
            assert_eq!(state.to_string().parse::<JobState>().unwrap(), state);
        }
        assert_eq!(
            "paused".parse::<JobState>().unwrap_err(),
            "unknown job state \"paused\""
        );
    }

    #[test]
    fn job_state_serialization_uses_uppercase_names() {
        assert_eq!(
            serde_json::to_string(&JobState::Cancelling).unwrap(),
            "\"CANCELLING\""
        );
        assert_eq!(
            serde_json::from_str::<JobState>("\"SUCCEEDED\"").unwrap(),
            JobState::Succeeded
        );
    }
}
