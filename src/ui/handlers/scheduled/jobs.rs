use std::path::PathBuf;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::adapters::SystemWorkingDirectoryResolver;
use crate::application::{self, CommitSelection, CreateJobInput};
use crate::domain::flow::{
    ExecutionMode, ScheduleSpec, parse_daily, parse_every, parse_first_at, parse_once,
};
use crate::store::StoreError;

use super::super::super::dto::JobDto;
use super::super::super::dto::scheduled::{
    ScheduledJobDto, ScheduledOccurrenceDto, ScheduledScheduleDto, StandaloneDefinitionDto,
};
use super::super::super::error::ApiError;
use super::super::super::state::ApiState;
use super::super::json_body;
use super::super::workspace::require_workspace_mode;

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct CreateJobRequest {
    user: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    cwd: String,
    command: String,
    schedule: ScheduledScheduleDto,
    retry: u32,
}

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct RevisionRequest {
    expected_draft_revision: i64,
}

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct ScheduleRequest {
    schedule: ScheduledScheduleDto,
    expected_draft_revision: i64,
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::ui) struct RunRequest {
    #[serde(default)]
    replace_next: bool,
}

pub(in crate::ui) async fn list(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let jobs = state
        .store
        .list_jobs_for_mode(None, None, Some(ExecutionMode::Scheduled))
        .map_err(store_error)?;
    let jobs = jobs
        .into_iter()
        .map(|job| scheduled_job(&state, job.id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({"jobs": jobs})))
}

pub(in crate::ui) async fn create(
    State(state): State<ApiState>,
    body: Result<Json<CreateJobRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    let schedule = parse_schedule(body.schedule)?;
    let job = application::jobs::create_job(
        &state.store,
        &SystemWorkingDirectoryResolver,
        CreateJobInput {
            user: body.user,
            name: body.name,
            description: body.description,
            cwd: request_cwd(body.cwd)?,
            command_line: body.command,
        },
    )?;
    state
        .store
        .configure_standalone(job.id, ExecutionMode::Scheduled, Some(schedule), body.retry)
        .map_err(store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"job": scheduled_job(&state, job.id)?})),
    ))
}

pub(in crate::ui) async fn detail(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    Ok(Json(
        json!({"job": scheduled_job(&state, job_id(&raw_id)?)?}),
    ))
}

pub(in crate::ui) async fn commit(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    scheduled_job(&state, id)?;
    application::jobs::commit_jobs(&state.scheduler, CommitSelection::Jobs(vec![id])).await?;
    Ok(Json(json!({"job": scheduled_job(&state, id)?})))
}

pub(in crate::ui) async fn freeze(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    state
        .store
        .freeze_scheduled_flow(&standalone_flow_id(id))
        .map_err(store_error)?;
    Ok(Json(json!({"job": scheduled_job(&state, id)?})))
}

pub(in crate::ui) async fn apply(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
    body: Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    let body = json_body(body)?;
    state
        .store
        .apply_scheduled_flow_draft(&standalone_flow_id(id), body.expected_draft_revision)
        .map_err(store_error)?;
    Ok(Json(json!({"job": scheduled_job(&state, id)?})))
}

pub(in crate::ui) async fn discard(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
    body: Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    let body = json_body(body)?;
    state
        .store
        .discard_scheduled_flow_draft(&standalone_flow_id(id), body.expected_draft_revision)
        .map_err(store_error)?;
    Ok(Json(json!({"job": scheduled_job(&state, id)?})))
}

pub(in crate::ui) async fn enable(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    set_enabled(state, raw_id, true).await
}

pub(in crate::ui) async fn disable(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    set_enabled(state, raw_id, false).await
}

pub(in crate::ui) async fn update_schedule(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
    body: Result<Json<ScheduleRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    let body = json_body(body)?;
    state
        .store
        .set_scheduled_flow_schedule_draft(
            &standalone_flow_id(id),
            parse_schedule(body.schedule)?,
            body.expected_draft_revision,
        )
        .map_err(store_error)?;
    Ok(Json(json!({"job": scheduled_job(&state, id)?})))
}

pub(in crate::ui) async fn occurrences(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    scheduled_job(&state, id)?;
    let occurrences = state
        .store
        .list_occurrences(&standalone_flow_id(id))
        .map_err(store_error)?;
    Ok(Json(json!({
        "occurrences": occurrences.into_iter().map(ScheduledOccurrenceDto::from).collect::<Vec<_>>()
    })))
}

pub(in crate::ui) async fn runs(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    scheduled_job(&state, id)?;
    let runs = state
        .store
        .list_flow_runs(&standalone_flow_id(id))
        .map_err(store_error)?;
    Ok(Json(json!({"runs": runs})))
}

pub(in crate::ui) async fn create_run(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
    body: Result<Json<RunRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    scheduled_job(&state, id)?;
    let body = json_body(body)?;
    let run = state
        .store
        .create_flow_run(&standalone_flow_id(id), "MANUAL", body.replace_next, None)
        .map_err(store_error)?;
    Ok((StatusCode::CREATED, Json(json!({"run": run}))))
}

async fn set_enabled(
    state: ApiState,
    raw_id: String,
    enabled: bool,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let id = job_id(&raw_id)?;
    state
        .store
        .set_scheduled_flow_enabled(&standalone_flow_id(id), enabled)
        .map_err(store_error)?;
    Ok(Json(json!({"job": scheduled_job(&state, id)?})))
}

fn scheduled(state: &ApiState) -> Result<(), ApiError> {
    require_workspace_mode(state, ExecutionMode::Scheduled)
}

fn job_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|error| ApiError::invalid_path("job id", raw, error))
}

fn standalone_flow_id(job_id: Uuid) -> String {
    format!("standalone/{job_id}")
}

fn request_cwd(cwd: String) -> Result<PathBuf, ApiError> {
    let cwd = PathBuf::from(cwd);
    if cwd.is_absolute() {
        Ok(cwd)
    } else {
        std::env::current_dir()
            .map(|root| root.join(cwd))
            .map_err(ApiError::internal)
    }
}

fn scheduled_job(state: &ApiState, job_id: Uuid) -> Result<ScheduledJobDto, ApiError> {
    let job = state.store.get_job(job_id).map_err(store_error)?;
    let definition = state
        .store
        .standalone_definition(job_id)
        .map_err(store_error)?;
    if definition.mode != ExecutionMode::Scheduled {
        return Err(ApiError::not_found("scheduled job does not exist"));
    }
    Ok(ScheduledJobDto {
        job: JobDto::from(job),
        definition: StandaloneDefinitionDto::from(definition),
        flow_id: standalone_flow_id(job_id),
    })
}

fn parse_schedule(schedule: ScheduledScheduleDto) -> Result<ScheduleSpec, ApiError> {
    match schedule {
        ScheduledScheduleDto::Once { at } => parse_once(&at)
            .map(|at| ScheduleSpec::Once { at })
            .map_err(ApiError::invalid_input),
        ScheduledScheduleDto::Daily { time, timezone } => parse_daily(&time)
            .map(|time| ScheduleSpec::Daily { time, timezone })
            .map_err(ApiError::invalid_input),
        ScheduledScheduleDto::Periodic { every, first_at } => Ok(ScheduleSpec::Periodic {
            every: parse_every(&every).map_err(ApiError::invalid_input)?,
            first_at: first_at
                .as_deref()
                .map(parse_first_at)
                .transpose()
                .map_err(ApiError::invalid_input)?,
        }),
    }
}

fn store_error(error: StoreError) -> ApiError {
    match error {
        StoreError::ScheduledModeChanged { actual } => ApiError::mode_changed(actual),
        StoreError::DraftRevisionConflict { current, .. } => {
            ApiError::conflict("draft revision conflict")
                .with_details(json!({"current_draft_revision": current}))
        }
        StoreError::NotFound { .. }
        | StoreError::Database(rusqlite::Error::QueryReturnedNoRows) => {
            ApiError::not_found("scheduled job does not exist")
        }
        StoreError::QueueLocked | StoreError::QueueUnlocked | StoreError::InvalidData(_) => {
            ApiError::conflict(error.to_string())
        }
        other => ApiError::internal(other),
    }
}
