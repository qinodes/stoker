use std::path::PathBuf;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::adapters::SystemWorkingDirectoryResolver;
use crate::application::{self, CommitSelection, CreateJobInput, DescriptionUpdate, JobFilter};
use crate::domain::JobState;

use super::super::dto::{
    CleanResponse, CreateJobRequest, CreateJobResponse, JobActionResponse, JobDetailResponse,
    JobDto, JobsResponse, LogsResponse, TimezoneResponse, UpdateDescriptionRequest,
};
use super::super::error::ApiError;
use super::super::state::ApiState;
use super::json_body;

const MAX_LOG_BYTES: usize = 256 * 1024;

#[derive(Debug, Default, Deserialize)]
pub(in crate::ui) struct JobsQuery {
    user: Option<String>,
    state: Option<String>,
}

pub(in crate::ui) async fn list(
    State(state): State<ApiState>,
    Query(query): Query<JobsQuery>,
) -> Result<Json<JobsResponse>, ApiError> {
    let job_state = query
        .state
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_uppercase().parse::<JobState>())
        .transpose()
        .map_err(ApiError::invalid_input)?;
    let jobs = application::jobs::query_jobs(
        &state.store,
        &JobFilter {
            user: query.user.filter(|value| !value.is_empty()),
            state: job_state,
        },
    )?;
    let timezone =
        crate::config::resolve_timezone(&state.paths, None).map_err(ApiError::internal)?;
    Ok(Json(JobsResponse {
        jobs: jobs.into_iter().map(JobDto::from).collect(),
        timezone: TimezoneResponse::from(&timezone),
    }))
}

pub(in crate::ui) async fn create(
    State(state): State<ApiState>,
    body: Result<Json<CreateJobRequest>, JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<CreateJobResponse>), ApiError> {
    let body = json_body(body)?;
    let job = application::jobs::create_job(
        &state.store,
        &SystemWorkingDirectoryResolver,
        CreateJobInput {
            user: body.user,
            name: body.name,
            cwd: PathBuf::from(body.cwd),
            command_line: body.command,
            description: body.description,
        },
    )?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateJobResponse {
            job: JobDto::from(job),
        }),
    ))
}

pub(in crate::ui) async fn detail(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<JobDetailResponse>, ApiError> {
    let id = parse_id("job id", &raw_id)?;
    let job = application::jobs::job_detail(&state.store, id)?;
    let working_directory_status = working_directory_status(job.state);
    let timezone =
        crate::config::resolve_timezone(&state.paths, None).map_err(ApiError::internal)?;
    Ok(Json(JobDetailResponse {
        job: JobDto::from(job),
        working_directory_status,
        display_timezone: timezone.name,
    }))
}

pub(in crate::ui) async fn update_description(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
    body: Result<Json<UpdateDescriptionRequest>, JsonRejection>,
) -> Result<Json<JobActionResponse>, ApiError> {
    let id = parse_id("job id", &raw_id)?;
    let body = json_body(body)?;
    let job = application::jobs::update_description(
        &state.store,
        DescriptionUpdate {
            id,
            description: body.description,
            expected_revision: body.expected_revision,
        },
    )?;
    Ok(Json(JobActionResponse {
        job: JobDto::from(job),
    }))
}

pub(in crate::ui) async fn commit(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<JobActionResponse>, ApiError> {
    let id = parse_id("job id", &raw_id)?;
    application::jobs::job_detail(&state.store, id)?;
    let job = application::jobs::commit_jobs(&state.scheduler, CommitSelection::Jobs(vec![id]))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal("scheduler returned no committed job"))?;
    Ok(Json(JobActionResponse {
        job: JobDto::from(job),
    }))
}

pub(in crate::ui) async fn cancel(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<JobActionResponse>, ApiError> {
    let id = parse_id("job id", &raw_id)?;
    application::jobs::job_detail(&state.store, id)?;
    let job = application::jobs::cancel_job(&state.scheduler, id).await?;
    Ok(Json(JobActionResponse {
        job: JobDto::from(job),
    }))
}

pub(in crate::ui) async fn clean(
    State(state): State<ApiState>,
) -> Result<Json<CleanResponse>, ApiError> {
    let jobs = application::jobs::clean_jobs(&state.store, &state.paths)?;
    Ok(Json(CleanResponse {
        removed: jobs.len(),
    }))
}

pub(in crate::ui) async fn logs(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<LogsResponse>, ApiError> {
    let id = parse_id("job id", &raw_id)?;
    let logs = application::logs::read_logs(&state.store, &state.paths, id, Some(MAX_LOG_BYTES))?;
    Ok(Json(LogsResponse {
        job: JobDto::from(logs.job),
        stdout: String::from_utf8_lossy(&logs.stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&logs.stderr.bytes).into_owned(),
        stdout_available: logs.stdout.available,
        stderr_available: logs.stderr.available,
        stdout_truncated: logs.stdout.truncated,
        stderr_truncated: logs.stderr.truncated,
        message: logs.message,
    }))
}

pub(in crate::ui) fn parse_id(kind: &'static str, raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|error| ApiError::invalid_path(kind, raw, error))
}

fn working_directory_status(state: JobState) -> &'static str {
    match state {
        JobState::Draft | JobState::Queued => "planned",
        JobState::Starting | JobState::Running | JobState::Cancelling => "active",
        _ => "source directory retained",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_working_directory_states_cover_boundaries() {
        let id = Uuid::new_v4();
        assert_eq!(parse_id("job id", &id.to_string()).unwrap(), id);
        assert!(parse_id("job id", "not-a-uuid").is_err());
        assert_eq!(working_directory_status(JobState::Draft), "planned");
        assert_eq!(working_directory_status(JobState::Running), "active");
        assert_eq!(
            working_directory_status(JobState::Succeeded),
            "source directory retained"
        );
    }
}
