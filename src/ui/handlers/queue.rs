use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};

use crate::application::{self, QueueMove};

use super::super::dto::{JobDto, QueueMoveRequest, QueueResponse};
use super::super::error::ApiError;
use super::super::state::ApiState;
use super::jobs::parse_id;
use super::json_body;

pub(in crate::ui) async fn get(
    State(state): State<ApiState>,
) -> Result<Json<QueueResponse>, ApiError> {
    let snapshot = application::queue::queue_snapshot(&state.store)?;
    Ok(Json(response(snapshot.jobs, snapshot.locked)))
}

pub(in crate::ui) async fn lock(
    State(state): State<ApiState>,
) -> Result<Json<QueueResponse>, ApiError> {
    set_locked(&state, true).await
}

pub(in crate::ui) async fn unlock(
    State(state): State<ApiState>,
) -> Result<Json<QueueResponse>, ApiError> {
    set_locked(&state, false).await
}

async fn set_locked(state: &ApiState, locked: bool) -> Result<Json<QueueResponse>, ApiError> {
    let result =
        application::queue::set_queue_locked(&state.store, &state.scheduler, locked).await?;
    Ok(Json(response(
        result.after.snapshot.jobs,
        result.after.snapshot.locked,
    )))
}

pub(in crate::ui) async fn move_job(
    State(state): State<ApiState>,
    Path(raw_id): Path<String>,
    body: Result<Json<QueueMoveRequest>, JsonRejection>,
) -> Result<Json<QueueResponse>, ApiError> {
    let id = parse_id("queue job id", &raw_id)?;
    let body = json_body(body)?;
    let result = application::queue::move_queued(
        &state.store,
        &state.scheduler,
        QueueMove {
            id,
            target_order: body.target_order,
        },
    )
    .await?;
    Ok(Json(response(result.snapshot.jobs, result.snapshot.locked)))
}

fn response(jobs: Vec<crate::domain::Job>, locked: bool) -> QueueResponse {
    QueueResponse {
        jobs: jobs.into_iter().map(JobDto::from).collect(),
        locked,
    }
}
