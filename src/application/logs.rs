//! Log use cases. Bounded reads and live streaming are intentionally distinct.

use uuid::Uuid;

use crate::application::error::ApplicationResult;
use crate::application::model::{FlowAttemptLogs, JobLogs, OutputStream};
use crate::application::ports::{
    FlowAttemptArtifacts, JobArtifacts, JobQueries, LogEventStream, SchedulerLogGateway,
};
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

/// Read the persisted output for one Flow task attempt.  This mirrors normal
/// job log semantics: numbered segments are combined in order and a bounded
/// request returns the newest bytes without materialising an unbounded file.
pub fn read_flow_attempt_logs(
    paths: &impl FlowAttemptArtifacts,
    run_id: Uuid,
    task_id: &str,
    attempt: u32,
    max_bytes: Option<usize>,
) -> ApplicationResult<FlowAttemptLogs> {
    let stdout =
        paths.read_flow_attempt_log(run_id, task_id, attempt, OutputStream::Stdout, max_bytes)?;
    let stderr =
        paths.read_flow_attempt_log(run_id, task_id, attempt, OutputStream::Stderr, max_bytes)?;
    let message = (!stdout.available && !stderr.available).then(|| {
        format!(
            "No logs are available for flow run {run_id}, task {task_id}, attempt {attempt} yet."
        )
    });
    Ok(FlowAttemptLogs {
        stdout,
        stderr,
        message,
    })
}
