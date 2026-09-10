use axum::Json;
use axum::extract::State;

use crate::domain::{MAX_JOB_DESCRIPTION_LENGTH, MAX_JOB_NAME_LENGTH, MAX_JOB_USER_LENGTH};

use super::super::dto::UiConfigResponse;
use super::super::error::ApiError;
use super::super::state::ApiState;

pub(in crate::ui) async fn ui_config(State(_state): State<ApiState>) -> Json<UiConfigResponse> {
    Json(UiConfigResponse {
        version: env!("CARGO_PKG_VERSION"),
        max_job_name_length: MAX_JOB_NAME_LENGTH,
        max_job_user_length: MAX_JOB_USER_LENGTH,
        max_job_description_length: MAX_JOB_DESCRIPTION_LENGTH,
    })
}

pub(in crate::ui) async fn shutdown(
    State(state): State<ApiState>,
) -> Result<&'static str, ApiError> {
    state.shutdown.notify_waiters();
    Ok("stopping")
}

pub(in crate::ui) async fn api_not_found() -> ApiError {
    ApiError::not_found("UI route not found")
}

pub(in crate::ui) async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}
