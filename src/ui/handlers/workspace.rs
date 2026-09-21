use axum::Json;
use axum::extract::State;
use chrono::Utc;

use crate::application;
use crate::domain::flow::ExecutionMode;

use super::super::dto::{TimezoneResponse, WorkspaceResponse, WorkspaceSchedulerResponse};
use super::super::error::ApiError;
use super::super::state::ApiState;

pub(in crate::ui) async fn workspace(
    State(state): State<ApiState>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let mode = state.store.current_mode().map_err(ApiError::internal)?;
    let recovery_fence = state
        .store
        .queue_recovery_fence()
        .map_err(ApiError::internal)?;
    let timezone =
        crate::config::resolve_timezone(&state.paths, None).map_err(ApiError::internal)?;
    let queue = application::queue::queue_status(&state.store, &state.scheduler).await?;
    let scheduler = match queue.scheduler {
        Some(status) => WorkspaceSchedulerResponse {
            running: true,
            pid: Some(status.pid),
            active_job: status.active_job,
            queued_jobs: status.queued_jobs,
        },
        None => WorkspaceSchedulerResponse {
            running: false,
            pid: None,
            active_job: None,
            queued_jobs: queue.snapshot.jobs.len(),
        },
    };
    Ok(Json(WorkspaceResponse {
        mode: mode_name(mode),
        queue_locked: queue.snapshot.locked,
        recovery_fence,
        scheduler,
        timezone: TimezoneResponse::from(&timezone),
        generated_at: Utc::now().to_rfc3339(),
    }))
}

#[allow(dead_code)] // Used by scheduled-mode routes added after this workspace contract.
pub(in crate::ui) fn require_workspace_mode(
    state: &ApiState,
    expected: ExecutionMode,
) -> Result<(), ApiError> {
    let actual = state.store.current_mode().map_err(ApiError::internal)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ApiError::mode_changed(actual))
    }
}

fn mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Serial => "serial",
        ExecutionMode::Scheduled => "scheduled",
    }
}
