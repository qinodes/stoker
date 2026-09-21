use std::path::PathBuf;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::domain::flow::{
    Dependency, DependencyMode, ExecutionMode, ScheduleSpec, parse_daily, parse_every,
    parse_first_at, parse_once,
};
use crate::store::{FlowTaskInput, StoreError};

use super::super::super::dto::scheduled::{
    ScheduledFlowDto, ScheduledOccurrenceDto, ScheduledScheduleDto,
};
use super::super::super::error::ApiError;
use super::super::super::state::ApiState;
use super::super::json_body;
use super::super::workspace::require_workspace_mode;

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct CreateFlowRequest {
    flow_id: String,
    name: String,
    owner: String,
    schedule: ScheduledScheduleDto,
}

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct TaskRequest {
    task_id: String,
    name: String,
    cwd: String,
    command: String,
    retry: u32,
    #[serde(default)]
    dependencies: Vec<Dependency>,
    #[serde(default)]
    depend_mode: DependencyMode,
    expected_draft_revision: i64,
}

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct TaskPatchRequest {
    command: Option<String>,
    cwd: Option<String>,
    retry: Option<u32>,
    dependencies: Option<Vec<Dependency>>,
    depend_mode: Option<DependencyMode>,
    expected_draft_revision: i64,
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
    let flows = state.store.list_flows(None).map_err(store_error)?;
    Ok(Json(
        json!({"flows": flows.into_iter().map(ScheduledFlowDto::from).collect::<Vec<_>>() }),
    ))
}

pub(in crate::ui) async fn create(
    State(state): State<ApiState>,
    body: Result<Json<CreateFlowRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    let schedule = parse_schedule(body.schedule)?;
    let flow = state
        .store
        .create_scheduled_flow(body.flow_id, body.name, body.owner, schedule)
        .map_err(store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"flow": ScheduledFlowDto::from(flow)})),
    ))
}

pub(in crate::ui) async fn detail(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let flow = state.store.get_flow(&flow_id).map_err(store_error)?;
    Ok(Json(json!({"flow": ScheduledFlowDto::from(flow)})))
}

pub(in crate::ui) async fn delete_draft(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    scheduled(&state)?;
    state
        .store
        .delete_scheduled_draft_flow(&flow_id)
        .map_err(store_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(in crate::ui) async fn commit(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    flow_response(
        state
            .store
            .commit_scheduled_flow(&flow_id)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn freeze(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    flow_response(
        state
            .store
            .freeze_scheduled_flow(&flow_id)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn apply(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
    body: Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    flow_response(
        state
            .store
            .apply_scheduled_flow_draft(&flow_id, body.expected_draft_revision)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn discard(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
    body: Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    flow_response(
        state
            .store
            .discard_scheduled_flow_draft(&flow_id, body.expected_draft_revision)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn enable(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    flow_response(
        state
            .store
            .set_scheduled_flow_enabled(&flow_id, true)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn disable(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    flow_response(
        state
            .store
            .set_scheduled_flow_enabled(&flow_id, false)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn update_schedule(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
    body: Result<Json<ScheduleRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    flow_response(
        state
            .store
            .set_scheduled_flow_schedule_draft(
                &flow_id,
                parse_schedule(body.schedule)?,
                body.expected_draft_revision,
            )
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn add_task(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
    body: Result<Json<TaskRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    flow_response(
        state
            .store
            .add_scheduled_flow_task(
                FlowTaskInput {
                    flow_id,
                    task_id: body.task_id,
                    name: body.name,
                    cwd: body.cwd,
                    command: body.command,
                    retry: body.retry,
                    dependencies: body.dependencies,
                    depend_mode: body.depend_mode,
                },
                body.expected_draft_revision,
            )
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn patch_task(
    State(state): State<ApiState>,
    Path((flow_id, task_id)): Path<(String, String)>,
    body: Result<Json<TaskPatchRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    flow_response(
        state
            .store
            .set_scheduled_flow_task_draft(
                &flow_id,
                &task_id,
                body.command.as_deref(),
                body.cwd.as_deref().map(PathBuf::from).as_deref(),
                body.retry,
                body.dependencies,
                body.depend_mode,
                body.expected_draft_revision,
            )
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn delete_task(
    State(state): State<ApiState>,
    Path((flow_id, task_id)): Path<(String, String)>,
    body: Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    flow_response(
        state
            .store
            .remove_scheduled_flow_task(&flow_id, &task_id, body.expected_draft_revision)
            .map_err(store_error)?,
    )
}

pub(in crate::ui) async fn occurrences(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let occurrences = state
        .store
        .list_occurrences(&flow_id)
        .map_err(store_error)?;
    Ok(Json(json!({
        "occurrences": occurrences.into_iter().map(ScheduledOccurrenceDto::from).collect::<Vec<_>>()
    })))
}

pub(in crate::ui) async fn runs(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let runs = state.store.list_flow_runs(&flow_id).map_err(store_error)?;
    Ok(Json(json!({"runs": runs})))
}

pub(in crate::ui) async fn create_run(
    State(state): State<ApiState>,
    Path(flow_id): Path<String>,
    body: Result<Json<RunRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    let run = state
        .store
        .create_flow_run(&flow_id, "MANUAL", body.replace_next, None)
        .map_err(store_error)?;
    Ok((StatusCode::CREATED, Json(json!({"run": run}))))
}

fn scheduled(state: &ApiState) -> Result<(), ApiError> {
    require_workspace_mode(state, ExecutionMode::Scheduled)
}

fn flow_response(
    flow: crate::domain::flow::FlowDefinition,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(json!({"flow": ScheduledFlowDto::from(flow)})))
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
        StoreError::QueueLocked | StoreError::QueueUnlocked | StoreError::InvalidData(_) => {
            ApiError::conflict(error.to_string())
        }
        StoreError::Database(rusqlite::Error::QueryReturnedNoRows) => {
            ApiError::not_found("flow does not exist")
        }
        other => ApiError::internal(other),
    }
}
