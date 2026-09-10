//! Log use cases. Bounded reads and live streaming are intentionally distinct.

use uuid::Uuid;

use crate::application::error::ApplicationResult;
use crate::application::model::{JobLogs, OutputStream};
use crate::application::ports::{JobArtifacts, JobQueries, LogEventStream, SchedulerLogGateway};
use crate::domain::JobState;

pub fn read_logs(
    repository: &impl JobQueries,
    artifacts: &impl JobArtifacts,
    id: Uuid,
    max_bytes: Option<usize>,
) -> ApplicationResult<JobLogs> {
    let job = repository.get_job(id)?;
    let stdout = artifacts.read_log(id, OutputStream::Stdout, max_bytes)?;
    let stderr = artifacts.read_log(id, OutputStream::Stderr, max_bytes)?;
    let available = stdout.available || stderr.available;
    let message = if job.state == JobState::Draft {
        Some(format!(
            "Job {id} is still DRAFT; commit it before the scheduler can create logs."
        ))
    } else if job.state == JobState::Queued {
        Some(format!(
            "Job {id} is QUEUED; logs will be available after the scheduler starts it."
        ))
    } else if !available {
        Some(format!("No logs are available for job {id} yet."))
    } else {
        None
    };
    Ok(JobLogs {
        job,
        stdout,
        stderr,
        message,
    })
}

pub async fn follow_logs(
    scheduler: &impl SchedulerLogGateway,
    id: Uuid,
) -> ApplicationResult<LogEventStream> {
    scheduler.follow_logs(id).await.map_err(Into::into)
}
