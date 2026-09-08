use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum number of Unicode characters allowed in a user-provided job name.
pub const MAX_JOB_NAME_LENGTH: usize = 128;
pub const MAX_JOB_USER_LENGTH: usize = 50;

pub fn validate_job_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("name must not be empty".to_owned());
    }
    if name.chars().count() > MAX_JOB_NAME_LENGTH {
        return Err(format!(
            "name must be {MAX_JOB_NAME_LENGTH} characters or fewer"
        ));
    }
    Ok(())
}

pub fn validate_job_user(user: &str) -> Result<(), String> {
    if user.trim().is_empty() {
        return Err("user must not be empty".to_owned());
    }
    if user.chars().count() > MAX_JOB_USER_LENGTH {
        return Err(format!(
            "user must be {MAX_JOB_USER_LENGTH} characters or fewer"
        ));
    }
    Ok(())
}

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
    pub fn as_str(self) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::JobState;
    use std::str::FromStr;

    #[test]
    fn job_states_round_trip_through_display_and_parser() {
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
    }

    #[test]
    fn unknown_job_state_reports_the_input() {
        let error = JobState::from_str("paused").unwrap_err();
        assert_eq!(error, "unknown job state \"paused\"");
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
