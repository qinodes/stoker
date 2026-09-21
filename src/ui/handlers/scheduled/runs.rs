use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::application;
use crate::domain::flow::{ExecutionMode, FlowRun};
use crate::store::StoreError;

use super::super::super::dto::scheduled::{ScheduledRunDto, ScheduledTaskRunDto};
use super::super::super::error::ApiError;
use super::super::super::state::ApiState;
use super::super::json_body;
use super::super::workspace::require_workspace_mode;

const MAX_LOG_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
pub(in crate::ui) struct MaxConcurrencyRequest {
    value: u32,
}

pub(in crate::ui) async fn detail(
    State(state): State<ApiState>,
    Path(raw_run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let run = state
        .store
        .get_flow_run(run_id(&raw_run_id)?)
        .map_err(store_error)?;
    Ok(Json(json!({"run": run_dto(&state, run)?})))
}

pub(in crate::ui) async fn cancel(
    State(state): State<ApiState>,
    Path(raw_run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let run_id = run_id(&raw_run_id)?;
    let run = state.store.get_flow_run(run_id).map_err(store_error)?;
    let cancelled = state
        .store
        .cancel_flow_run(&run.flow_id, run_id)
        .map_err(store_error)?;
    Ok(Json(json!({"run": run_dto(&state, cancelled)?})))
}

pub(in crate::ui) async fn cancel_task(
    State(state): State<ApiState>,
    Path((raw_run_id, task_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let run_id = run_id(&raw_run_id)?;
    let run = state.store.get_flow_run(run_id).map_err(store_error)?;
    let cancelled = state
        .store
        .cancel_flow_task(&run.flow_id, &task_id, run_id)
        .map_err(store_error)?;
    Ok(Json(json!({"run": run_dto(&state, cancelled)?})))
}

pub(in crate::ui) async fn logs(
    State(state): State<ApiState>,
    Path((raw_run_id, task_id, attempt)): Path<(String, String, u32)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let run_id = run_id(&raw_run_id)?;
    state.store.get_flow_run(run_id).map_err(store_error)?;
    let exists = state
        .store
        .list_flow_attempts(run_id, &task_id)
        .map_err(store_error)?
        .into_iter()
        .any(|item| item.number == attempt);
    if !exists {
        return Err(ApiError::not_found("flow task attempt does not exist"));
    }
    let logs = application::logs::read_flow_attempt_logs(
        &state.paths,
        run_id,
        &task_id,
        attempt,
        Some(MAX_LOG_BYTES),
    )
    .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "stdout": String::from_utf8_lossy(&logs.stdout.bytes),
        "stderr": String::from_utf8_lossy(&logs.stderr.bytes),
        "stdout_available": logs.stdout.available,
        "stderr_available": logs.stderr.available,
        "stdout_truncated": logs.stdout.truncated,
        "stderr_truncated": logs.stderr.truncated,
        "stdout_capture_error": logs.stdout.capture_error,
        "stderr_capture_error": logs.stderr.capture_error,
        "message": logs.message,
    })))
}

pub(in crate::ui) async fn set_max_concurrency(
    State(state): State<ApiState>,
    body: Result<Json<MaxConcurrencyRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    if body.value == 0 {
        return Err(ApiError::invalid_input(
            "max concurrency must be a positive integer",
        ));
    }
    let value = state
        .store
        .set_scheduled_concurrency_for_scheduled_workspace(body.value)
        .map_err(store_error)?;
    Ok(Json(json!({"max_concurrency": value})))
}

pub(in crate::ui) async fn reconcile_recovery(
    State(state): State<ApiState>,
    Path(raw_run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    scheduled(&state)?;
    let run_id = run_id(&raw_run_id)?;
    state
        .store
        .reconcile_recovery(run_id, true)
        .map_err(store_error)?;
    let run = state.store.get_flow_run(run_id).map_err(store_error)?;
    Ok(Json(json!({"run": run_dto(&state, run)?})))
}

pub(in crate::ui) fn run_dto(state: &ApiState, run: FlowRun) -> Result<ScheduledRunDto, ApiError> {
    let tasks = run
        .tasks
        .iter()
        .map(|task| {
            state
                .store
                .list_flow_attempts(run.run_id, &task.task_id)
                .map(|attempts| ScheduledTaskRunDto::new(task, attempts))
                .map_err(store_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ScheduledRunDto::new(run, tasks))
}

fn scheduled(state: &ApiState) -> Result<(), ApiError> {
    require_workspace_mode(state, ExecutionMode::Scheduled)
}

fn run_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|error| ApiError::invalid_path("run id", raw, error))
}

fn store_error(error: StoreError) -> ApiError {
    match error {
        StoreError::ScheduledModeChanged { actual } => ApiError::mode_changed(actual),
        StoreError::Database(rusqlite::Error::QueryReturnedNoRows) => {
            ApiError::not_found("flow run does not exist")
        }
        StoreError::QueueLocked | StoreError::QueueUnlocked | StoreError::InvalidData(_) => {
            ApiError::conflict(error.to_string())
        }
        other => ApiError::internal(other),
    }
}
