use std::path::Path;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;

use crate::application::{self, SnapshotReason};

use super::super::dto::{
    ConfigurationResponse, RestoreRequest, SnapshotResponse, TimezoneRequest, TimezoneResponse,
};
use super::super::error::ApiError;
use super::super::state::ApiState;
use super::json_body;

pub(in crate::ui) async fn get(
    State(state): State<ApiState>,
) -> Result<Json<ConfigurationResponse>, ApiError> {
    configuration(&state).map(Json)
}

pub(in crate::ui) async fn set_timezone(
    State(state): State<ApiState>,
    body: Result<Json<TimezoneRequest>, JsonRejection>,
) -> Result<Json<ConfigurationResponse>, ApiError> {
    let body = json_body(body)?;
    application::configuration::set_timezone(&state.paths, body.value.trim().to_owned())?;
    configuration(&state).map(Json)
}

pub(in crate::ui) async fn unset_timezone(
    State(state): State<ApiState>,
) -> Result<Json<ConfigurationResponse>, ApiError> {
    application::configuration::unset_timezone(&state.paths)?;
    configuration(&state).map(Json)
}

pub(in crate::ui) async fn create_snapshot(
    State(state): State<ApiState>,
) -> Result<Json<ConfigurationResponse>, ApiError> {
    application::configuration::create_snapshot(&state.paths, SnapshotReason::Manual)?;
    configuration(&state).map(Json)
}

pub(in crate::ui) async fn restore_snapshot(
    State(state): State<ApiState>,
    body: Result<Json<RestoreRequest>, JsonRejection>,
) -> Result<Json<ConfigurationResponse>, ApiError> {
    let body = json_body(body)?;
    application::configuration::restore_snapshot(&state.paths, Path::new(&body.path))?;
    configuration(&state).map(Json)
}

fn configuration(state: &ApiState) -> Result<ConfigurationResponse, ApiError> {
    let config = application::configuration::configuration(&state.paths)?;
    let timezone =
        crate::config::resolve_timezone(&state.paths, None).map_err(ApiError::internal)?;
    let snapshots = application::configuration::list_snapshots(&state.paths)?
        .into_iter()
        .map(SnapshotResponse::from)
        .collect();
    Ok(ConfigurationResponse {
        config: crate::config::StokerConfig {
            timezone: config.timezone,
        },
        effective_timezone: TimezoneResponse::from(&timezone),
        timezones: chrono_tz::TZ_VARIANTS
            .iter()
            .map(ToString::to_string)
            .collect(),
        config_path: state.paths.config_path().to_string_lossy().into_owned(),
        snapshot_dir: state.paths.snapshot_dir().to_string_lossy().into_owned(),
        snapshots,
    })
}
