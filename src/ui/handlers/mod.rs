pub(super) mod configuration;
pub(super) mod filesystem;
pub(super) mod jobs;
pub(super) mod queue;
pub(super) mod status;
pub(super) mod system;

use axum::Json;
use axum::extract::rejection::JsonRejection;

use super::error::ApiError;

pub(super) fn json_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    body.map(|Json(value)| value)
        .map_err(ApiError::from_json_rejection)
}
