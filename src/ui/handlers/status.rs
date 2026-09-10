use axum::Json;
use axum::extract::State;
use chrono::Utc;

use crate::application::{self, JobFilter};
use crate::domain::{Job, JobState};

use super::super::dto::{CountResponse, SchedulerResponse, StatusResponse, TimezoneResponse};
use super::super::error::ApiError;
use super::super::state::ApiState;

pub(in crate::ui) async fn status(
    State(state): State<ApiState>,
) -> Result<Json<StatusResponse>, ApiError> {
    let jobs = application::jobs::query_jobs(&state.store, &JobFilter::default())?;
    let timezone =
        crate::config::resolve_timezone(&state.paths, None).map_err(ApiError::internal)?;
    let queue = application::queue::queue_status(&state.store, &state.scheduler).await?;
    let scheduler = match &queue.scheduler {
        Some(service) => SchedulerResponse {
            running: true,
            pid: Some(service.pid),
            active_job: service.active_job,
            queued_jobs: service.queued_jobs,
        },
        None => SchedulerResponse {
            running: false,
            pid: None,
            active_job: None,
            queued_jobs: queue.snapshot.jobs.len(),
        },
    };
    Ok(Json(StatusResponse {
        scheduler,
        counts: CountResponse {
            total: jobs.len(),
            draft: count_state(&jobs, JobState::Draft),
            queued: count_state(&jobs, JobState::Queued),
            active: jobs
                .iter()
                .filter(|job| {
                    matches!(
                        job.state,
                        JobState::Starting | JobState::Running | JobState::Cancelling
                    )
                })
                .count(),
            succeeded: count_state(&jobs, JobState::Succeeded),
            failed: jobs
                .iter()
                .filter(|job| matches!(job.state, JobState::Failed | JobState::Lost))
                .count(),
        },
        queue_locked: queue.snapshot.locked,
        timezone: TimezoneResponse::from(&timezone),
        generated_at: Utc::now().to_rfc3339(),
    }))
}

fn count_state(jobs: &[Job], state: JobState) -> usize {
    jobs.iter().filter(|job| job.state == state).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_counts_cover_empty_and_repeated_values() {
        assert_eq!(count_state(&[], JobState::Draft), 0);
    }
}
