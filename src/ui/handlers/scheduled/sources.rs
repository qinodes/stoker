use std::str::FromStr;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use serde::Deserialize;
use serde_json::Value;

use crate::domain::flow::ExecutionMode;
use crate::flow_source::{
    FlowSourceDocument, parse_document, resolve_workspace_document, write_snapshot,
};
use crate::store::{FlowSourceMode, StoreError};

use super::super::super::dto::scheduled::{
    FlowSourceExportResponse, FlowSourceSnapshotResponse, FlowSourceStateResponse,
    FlowSyncPreviewResponse,
};
use super::super::super::error::ApiError;
use super::super::super::state::ApiState;
use super::super::json_body;
use super::super::workspace::require_workspace_mode;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ui) struct SourceDocumentRequest {
    document: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ui) struct SourceSyncRequest {
    document: Value,
    confirmed_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ui) struct SourceModeRequest {
    mode: String,
}

pub(in crate::ui) async fn state(
    State(state): State<ApiState>,
) -> Result<Json<FlowSourceStateResponse>, ApiError> {
    scheduled(&state)?;
    state
        .store
        .flow_source_state()
        .map(FlowSourceStateResponse::from)
        .map(Json)
        .map_err(store_error)
}

pub(in crate::ui) async fn export(
    State(state): State<ApiState>,
) -> Result<Json<FlowSourceExportResponse>, ApiError> {
    scheduled(&state)?;
    let document = state.store.export_flow_source().map_err(store_error)?;
    Ok(Json(FlowSourceExportResponse { document }))
}

pub(in crate::ui) async fn set_mode(
    State(state): State<ApiState>,
    body: Result<Json<SourceModeRequest>, JsonRejection>,
) -> Result<Json<FlowSourceStateResponse>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    let mode = FlowSourceMode::from_str(&body.mode).map_err(ApiError::invalid_input)?;
    state
        .store
        .set_flow_source_mode(mode)
        .map(FlowSourceStateResponse::from)
        .map(Json)
        .map_err(store_error)
}

pub(in crate::ui) async fn dry_run(
    State(state): State<ApiState>,
    body: Result<Json<SourceDocumentRequest>, JsonRejection>,
) -> Result<Json<FlowSyncPreviewResponse>, ApiError> {
    scheduled(&state)?;
    let source = parse_source_document(json_body(body)?)?;
    sync_preview(&state, &source).map(Json)
}

pub(in crate::ui) async fn sync(
    State(state): State<ApiState>,
    body: Result<Json<SourceSyncRequest>, JsonRejection>,
) -> Result<Json<FlowSyncPreviewResponse>, ApiError> {
    scheduled(&state)?;
    let body = json_body(body)?;
    let source = parse_source_document(SourceDocumentRequest {
        document: body.document,
    })?;
    let preview = sync_preview(&state, &source)?;
    if body.confirmed_hash != preview.hash {
        return Err(ApiError::conflict(
            "confirmed preview hash does not match the recomputed Flow source hash",
        ));
    }
    let desired = resolve_workspace_document(&source, &state.paths.root)
        .map_err(|error| ApiError::invalid_input(error.to_string()))?;
    state
        .store
        .sync_flow_source(&state.paths.root, &source, &desired, false)
        .map(FlowSyncPreviewResponse::from)
        .map(Json)
        .map_err(store_error)
}

pub(in crate::ui) async fn snapshot(
    State(state): State<ApiState>,
) -> Result<Json<FlowSourceSnapshotResponse>, ApiError> {
    scheduled(&state)?;
    let document = state.store.export_flow_source().map_err(store_error)?;
    let path = write_snapshot(&state.paths.root, &document).map_err(ApiError::internal)?;
    Ok(Json(FlowSourceSnapshotResponse {
        document,
        path: path.to_string_lossy().into_owned(),
    }))
}

fn sync_preview(
    state: &ApiState,
    source: &FlowSourceDocument,
) -> Result<FlowSyncPreviewResponse, ApiError> {
    let desired = resolve_workspace_document(source, &state.paths.root)
        .map_err(|error| ApiError::invalid_input(error.to_string()))?;
    state
        .store
        .sync_flow_source(&state.paths.root, source, &desired, true)
        .map(FlowSyncPreviewResponse::from)
        .map_err(store_error)
}

fn parse_source_document(request: SourceDocumentRequest) -> Result<FlowSourceDocument, ApiError> {
    let bytes = serde_json::to_vec(&request.document)
        .map_err(|error| ApiError::invalid_input(error.to_string()))?;
    parse_document(&bytes).map_err(|error| ApiError::invalid_input(error.to_string()))
}

fn scheduled(state: &ApiState) -> Result<(), ApiError> {
    require_workspace_mode(state, ExecutionMode::Scheduled)
}

fn store_error(error: StoreError) -> ApiError {
    match error {
        StoreError::ScheduledModeChanged { actual } => ApiError::mode_changed(actual),
        StoreError::QueueLocked | StoreError::QueueUnlocked | StoreError::InvalidData(_) => {
            ApiError::conflict(error.to_string())
        }
        other => ApiError::internal(other),
    }
}
