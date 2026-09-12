use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::json;

use crate::application::{self, JobFilter};
use crate::config::{LogPolicy, POLICY_MB_BYTES, RuntimePolicy};
use crate::domain::{Job, JobState};
use crate::store::StoreError;

use super::super::dto::{
    LogPolicyUnitsResponse, PolicyActiveJobResponse, PolicyDefaultsResponse, PolicyResponse,
    PolicyUnitsResponse, PolicyValueRequest, RuntimePolicyUnitsResponse,
};
use super::super::error::{ApiError, ErrorCode};
use super::super::state::ApiState;
use super::json_body;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyKey {
    LogMaxBytesPerJob,
    LogSegmentBytes,
    LogMaxBytesTotal,
    LogRetentionJobs,
    LogDiskReserveBytes,
    TerminationGraceMs,
    MaxRuntimeMs,
    StartupTimeoutMs,
}

impl PolicyKey {
    fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "log-max-bytes-per-job" => Ok(Self::LogMaxBytesPerJob),
            "log-segment-bytes" => Ok(Self::LogSegmentBytes),
            "log-max-bytes-total" => Ok(Self::LogMaxBytesTotal),
            "log-retention-jobs" => Ok(Self::LogRetentionJobs),
            "log-disk-reserve-bytes" => Ok(Self::LogDiskReserveBytes),
            "termination-grace-ms" => Ok(Self::TerminationGraceMs),
            "max-runtime-ms" => Ok(Self::MaxRuntimeMs),
            "startup-timeout-ms" => Ok(Self::StartupTimeoutMs),
            _ => Err(ApiError::invalid_path(
                "policy key",
                value,
                "unknown policy key",
            )),
        }
    }

    fn is_log(self) -> bool {
        matches!(
            self,
            Self::LogMaxBytesPerJob
                | Self::LogSegmentBytes
                | Self::LogMaxBytesTotal
                | Self::LogRetentionJobs
                | Self::LogDiskReserveBytes
        )
    }

    fn is_byte(self) -> bool {
        matches!(
            self,
            Self::LogMaxBytesPerJob
                | Self::LogSegmentBytes
                | Self::LogMaxBytesTotal
                | Self::LogDiskReserveBytes
        )
    }

    fn name(self) -> &'static str {
        match self {
            Self::LogMaxBytesPerJob => "log-max-bytes-per-job",
            Self::LogSegmentBytes => "log-segment-bytes",
            Self::LogMaxBytesTotal => "log-max-bytes-total",
            Self::LogRetentionJobs => "log-retention-jobs",
            Self::LogDiskReserveBytes => "log-disk-reserve-bytes",
            Self::TerminationGraceMs => "termination-grace-ms",
            Self::MaxRuntimeMs => "max-runtime-ms",
            Self::StartupTimeoutMs => "startup-timeout-ms",
        }
    }
}

pub(in crate::ui) async fn get(
    State(state): State<ApiState>,
) -> Result<Json<PolicyResponse>, ApiError> {
    policy(&state).map(Json)
}

pub(in crate::ui) async fn set(
    State(state): State<ApiState>,
    Path(raw_key): Path<String>,
    body: Result<Json<PolicyValueRequest>, JsonRejection>,
) -> Result<Json<PolicyResponse>, ApiError> {
    let key = PolicyKey::parse(&raw_key)?;
    let body = json_body(body)?;
    let value = body.value.as_u64().ok_or_else(|| {
        ApiError::invalid_input(format!("{} must be a non-negative integer", key.name()))
    })?;

    if key.is_log() {
        let value = if key.is_byte() {
            value
                .checked_mul(POLICY_MB_BYTES)
                .ok_or_else(|| ApiError::invalid_input(format!("{} is too large", key.name())))?
        } else {
            value
        };
        let mut current = state.store.log_policy().map_err(store_error)?;
        assign_log_value(&mut current, key, value)?;
        state.store.set_log_policy(current).map_err(store_error)?;
    } else {
        let mut current = state.store.runtime_policy().map_err(store_error)?;
        assign_runtime_value(&mut current, key, Some(value))?;
        state
            .store
            .set_runtime_policy(current)
            .map_err(store_error)?;
    }
    policy(&state).map(Json)
}

pub(in crate::ui) async fn unset(
    State(state): State<ApiState>,
    Path(raw_key): Path<String>,
) -> Result<Json<PolicyResponse>, ApiError> {
    let key = PolicyKey::parse(&raw_key)?;
    if key.is_log() {
        let mut current = state.store.log_policy().map_err(store_error)?;
        assign_log_value(&mut current, key, log_value(LogPolicy::default(), key))?;
        state.store.set_log_policy(current).map_err(store_error)?;
    } else {
        let mut current = state.store.runtime_policy().map_err(store_error)?;
        assign_runtime_value(
            &mut current,
            key,
            runtime_value(RuntimePolicy::default(), key),
        )?;
        state
            .store
            .set_runtime_policy(current)
            .map_err(store_error)?;
    }
    policy(&state).map(Json)
}

fn policy(state: &ApiState) -> Result<PolicyResponse, ApiError> {
    let log = state.store.log_policy().map_err(store_error)?;
    let runtime = state.store.runtime_policy().map_err(store_error)?;
    let queue_locked = state.store.queue_locked().map_err(store_error)?;
    let jobs = application::jobs::query_jobs(&state.store, &JobFilter::default())?;
    let active_jobs = jobs
        .iter()
        .filter(|job| {
            matches!(
                job.state,
                JobState::Starting | JobState::Running | JobState::Cancelling
            )
        })
        .map(active_job)
        .collect::<Vec<_>>();
    let can_update = queue_locked && active_jobs.is_empty();
    let blocked_reason = match (queue_locked, active_jobs.is_empty()) {
        (false, false) => {
            Some("Lock the queue and wait for active jobs to finish before changing policy.".into())
        }
        (false, true) => Some("Lock the queue before changing policy.".into()),
        (true, false) => Some(
            "Wait for all STARTING, RUNNING, and CANCELLING jobs to finish before changing policy."
                .into(),
        ),
        (true, true) => None,
    };

    Ok(PolicyResponse {
        log: log.into(),
        runtime: runtime.into(),
        defaults: PolicyDefaultsResponse {
            log: LogPolicy::default().into(),
            runtime: RuntimePolicy::default().into(),
        },
        units: PolicyUnitsResponse {
            log: LogPolicyUnitsResponse {
                max_bytes_per_job: "MB",
                segment_bytes: "MB",
                max_bytes_total: "MB",
                retention_jobs: "jobs",
                disk_reserve_bytes: "MB",
            },
            runtime: RuntimePolicyUnitsResponse {
                termination_grace_ms: "milliseconds",
                max_runtime_ms: "milliseconds",
                startup_timeout_ms: "milliseconds",
            },
        },
        queue_locked,
        can_update,
        active_jobs,
        blocked_reason,
    })
}

fn active_job(job: &Job) -> PolicyActiveJobResponse {
    PolicyActiveJobResponse {
        id: job.id,
        name: job.name.clone(),
        state: job.state.into(),
    }
}

fn assign_log_value(policy: &mut LogPolicy, key: PolicyKey, value: u64) -> Result<(), ApiError> {
    match key {
        PolicyKey::LogMaxBytesPerJob => policy.max_bytes_per_job = value,
        PolicyKey::LogSegmentBytes => policy.segment_bytes = value,
        PolicyKey::LogMaxBytesTotal => policy.max_bytes_total = value,
        PolicyKey::LogRetentionJobs => policy.retention_jobs = value,
        PolicyKey::LogDiskReserveBytes => policy.disk_reserve_bytes = value,
        _ => {
            return Err(ApiError::invalid_input(format!(
                "{} is not a log policy value",
                key.name()
            )));
        }
    }
    Ok(())
}

fn assign_runtime_value(
    policy: &mut RuntimePolicy,
    key: PolicyKey,
    value: Option<u64>,
) -> Result<(), ApiError> {
    match key {
        PolicyKey::TerminationGraceMs => policy.termination_grace_ms = value.unwrap_or_default(),
        PolicyKey::MaxRuntimeMs => policy.max_runtime_ms = value,
        PolicyKey::StartupTimeoutMs => policy.startup_timeout_ms = value.unwrap_or_default(),
        _ => {
            return Err(ApiError::invalid_input(format!(
                "{} is not a runtime policy value",
                key.name()
            )));
        }
    }
    Ok(())
}

fn log_value(policy: LogPolicy, key: PolicyKey) -> u64 {
    match key {
        PolicyKey::LogMaxBytesPerJob => policy.max_bytes_per_job,
        PolicyKey::LogSegmentBytes => policy.segment_bytes,
        PolicyKey::LogMaxBytesTotal => policy.max_bytes_total,
        PolicyKey::LogRetentionJobs => policy.retention_jobs,
        PolicyKey::LogDiskReserveBytes => policy.disk_reserve_bytes,
        _ => 0,
    }
}

fn runtime_value(policy: RuntimePolicy, key: PolicyKey) -> Option<u64> {
    match key {
        PolicyKey::TerminationGraceMs => Some(policy.termination_grace_ms),
        PolicyKey::MaxRuntimeMs => policy.max_runtime_ms,
        PolicyKey::StartupTimeoutMs => Some(policy.startup_timeout_ms),
        _ => None,
    }
}

fn store_error(error: StoreError) -> ApiError {
    match error {
        StoreError::QueueUnlocked => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            "Lock the queue before changing policy.",
        )
        .with_details(json!({
            "queue_locked": false,
            "can_update": false,
            "action": "Use the queue lock control, then try again."
        })),
        StoreError::ActiveJob { id, state } => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            format!("Wait for job {id} ({state}) to finish before changing policy."),
        )
        .with_details(json!({
            "active_job": id,
            "state": state.as_str(),
            "can_update": false
        })),
        StoreError::InvalidData(message) => ApiError::invalid_input(message),
        other => ApiError::internal(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG_KEYS: [PolicyKey; 5] = [
        PolicyKey::LogMaxBytesPerJob,
        PolicyKey::LogSegmentBytes,
        PolicyKey::LogMaxBytesTotal,
        PolicyKey::LogRetentionJobs,
        PolicyKey::LogDiskReserveBytes,
    ];

    const RUNTIME_KEYS: [PolicyKey; 3] = [
        PolicyKey::TerminationGraceMs,
        PolicyKey::MaxRuntimeMs,
        PolicyKey::StartupTimeoutMs,
    ];

    #[test]
    fn policy_keys_parse_classify_and_map_to_stable_names() {
        let names = [
            "log-max-bytes-per-job",
            "log-segment-bytes",
            "log-max-bytes-total",
            "log-retention-jobs",
            "log-disk-reserve-bytes",
            "termination-grace-ms",
            "max-runtime-ms",
            "startup-timeout-ms",
        ];
        for (key, name) in LOG_KEYS.into_iter().chain(RUNTIME_KEYS).zip(names) {
            assert_eq!(PolicyKey::parse(name).unwrap(), key);
            assert_eq!(key.name(), name);
        }
        let error = PolicyKey::parse("unknown").unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.code, ErrorCode::InvalidInput);

        for key in LOG_KEYS {
            assert!(key.is_log());
        }
        for key in RUNTIME_KEYS {
            assert!(!key.is_log());
        }
        for key in [
            PolicyKey::LogMaxBytesPerJob,
            PolicyKey::LogSegmentBytes,
            PolicyKey::LogMaxBytesTotal,
            PolicyKey::LogDiskReserveBytes,
        ] {
            assert!(key.is_byte());
        }
        assert!(!PolicyKey::LogRetentionJobs.is_byte());
    }

    #[test]
    fn policy_value_helpers_cover_each_branch_and_reject_cross_section_keys() {
        let mut log = LogPolicy::default();
        for (index, key) in LOG_KEYS.into_iter().enumerate() {
            assign_log_value(&mut log, key, index as u64 + 1).unwrap();
            assert_eq!(log_value(log, key), index as u64 + 1);
        }
        for key in RUNTIME_KEYS {
            assert_eq!(log_value(log, key), 0);
            let error = assign_log_value(&mut log, key, 1).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidInput);
        }

        let mut runtime = RuntimePolicy::default();
        assign_runtime_value(&mut runtime, PolicyKey::TerminationGraceMs, None).unwrap();
        assign_runtime_value(&mut runtime, PolicyKey::MaxRuntimeMs, Some(10)).unwrap();
        assign_runtime_value(&mut runtime, PolicyKey::StartupTimeoutMs, Some(20)).unwrap();
        assert_eq!(runtime.termination_grace_ms, 0);
        assert_eq!(
            runtime_value(runtime, PolicyKey::TerminationGraceMs),
            Some(0)
        );
        assert_eq!(runtime_value(runtime, PolicyKey::MaxRuntimeMs), Some(10));
        assert_eq!(
            runtime_value(runtime, PolicyKey::StartupTimeoutMs),
            Some(20)
        );
        assert_eq!(runtime_value(runtime, PolicyKey::LogRetentionJobs), None);
        let error =
            assign_runtime_value(&mut runtime, PolicyKey::LogRetentionJobs, Some(1)).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn store_error_maps_data_and_unexpected_failures_to_api_errors() {
        let invalid = store_error(StoreError::InvalidData("bad value".into()));
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid.code, ErrorCode::InvalidInput);

        let internal = store_error(StoreError::Database(rusqlite::Error::InvalidQuery));
        assert_eq!(internal.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(internal.code, ErrorCode::Internal);
    }
}
