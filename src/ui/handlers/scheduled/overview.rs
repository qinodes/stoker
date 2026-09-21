use std::collections::BTreeSet;

use axum::Json;
use axum::extract::State;
use serde_json::json;

use crate::domain::flow::{ExecutionMode, FlowRunState, OccurrenceState};

use super::super::super::error::ApiError;
use super::super::super::state::ApiState;
use super::super::workspace::require_workspace_mode;

pub(in crate::ui) async fn overview(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_workspace_mode(&state, ExecutionMode::Scheduled)?;
    let capacity = json!({
        "max_concurrency": state.store.scheduled_concurrency().map_err(ApiError::internal)?,
        "active_attempts": state.store.flow_active_attempt_count().map_err(ApiError::internal)?,
    });
    let mut flow_ids = state
        .store
        .list_flows(None)
        .map_err(ApiError::internal)?
        .into_iter()
        .filter(|flow| flow.mode == ExecutionMode::Scheduled)
        .map(|flow| flow.flow_id)
        .collect::<BTreeSet<_>>();
    for job in state
        .store
        .list_jobs_for_mode(None, None, Some(ExecutionMode::Scheduled))
        .map_err(ApiError::internal)?
    {
        flow_ids.insert(format!("standalone/{}", job.id));
    }

    let mut active_runs = Vec::new();
    let mut next_occurrences = Vec::new();
    let mut recent_failures = Vec::new();
    for flow_id in flow_ids {
        for occurrence in state
            .store
            .list_occurrences(&flow_id)
            .map_err(ApiError::internal)?
        {
            if occurrence.state == OccurrenceState::Pending {
                next_occurrences.push(json!({
                    "flow_id": occurrence.flow_id,
                    "occurrence_id": occurrence.occurrence_id,
                    "due_at": occurrence.due_at.to_rfc3339(),
                }));
            }
        }
        for run in state
            .store
            .list_flow_runs(&flow_id)
            .map_err(ApiError::internal)?
        {
            let summary = json!({
                "run_id": run.run_id,
                "flow_id": run.flow_id,
                "state": run.state,
                "started_at": run.started_at.map(|value| value.to_rfc3339()),
                "finished_at": run.finished_at.map(|value| value.to_rfc3339()),
            });
            if !run.state.is_terminal() {
                active_runs.push(summary);
            } else if matches!(
                run.state,
                FlowRunState::Failed | FlowRunState::FailedToStart | FlowRunState::Lost
            ) {
                recent_failures.push(summary);
            }
        }
    }
    active_runs.sort_by_key(|item| item["started_at"].as_str().unwrap_or_default().to_owned());
    next_occurrences.sort_by_key(|item| item["due_at"].as_str().unwrap_or_default().to_owned());
    recent_failures.sort_by_key(|item| item["finished_at"].as_str().unwrap_or_default().to_owned());
    recent_failures.reverse();
    recent_failures.truncate(10);
    Ok(Json(json!({
        "capacity": capacity,
        "active_runs": active_runs,
        "next_occurrences": next_occurrences,
        "recent_failures": recent_failures,
    })))
}
