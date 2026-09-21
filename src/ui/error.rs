use std::path::Path;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

use crate::application::{self, ApplicationError, ApplicationErrorCode, Conflict};
use crate::domain::flow::ExecutionMode;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ErrorCode {
    InvalidInput,
    Forbidden,
    NotFound,
    Conflict,
    #[allow(dead_code)] // Reserved for scheduled-mode routes added after this workspace contract.
    ModeChanged,
    MethodNotAllowed,
    PayloadTooLarge,
    UnsupportedMediaType,
    ServiceUnavailable,
    Timeout,
    InvalidDependencyData,
    Internal,
}

#[derive(Debug, Serialize)]
pub(super) struct ErrorResponse {
    pub error: String,
    pub code: ErrorCode,
    pub message: String,
    pub details: Value,
}

#[derive(Debug)]
pub(super) struct ApiError {
    pub(super) status: StatusCode,
    pub(super) code: ErrorCode,
    message: String,
    details: Value,
}

impl ApiError {
    pub(super) fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: Value::Null,
        }
    }

    pub(super) fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub(super) fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidInput, message)
    }

    pub(super) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, message)
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, message)
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message)
    }

    pub(super) fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::MethodNotAllowed,
            "HTTP method is not allowed",
        )
    }

    #[allow(dead_code)] // Used by scheduled-mode routes added after this workspace contract.
    pub(super) fn mode_changed(actual: ExecutionMode) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::ModeChanged,
            format!("workspace mode changed to {actual}"),
        )
        .with_details(json!({"mode": actual.to_string()}))
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            error.to_string(),
        )
    }

    pub(super) fn from_json_rejection(rejection: JsonRejection) -> Self {
        let status = rejection.status();
        let code = match status {
            StatusCode::UNSUPPORTED_MEDIA_TYPE => ErrorCode::UnsupportedMediaType,
            StatusCode::PAYLOAD_TOO_LARGE => ErrorCode::PayloadTooLarge,
            _ => ErrorCode::InvalidInput,
        };
        Self::new(status, code, rejection.body_text())
    }

    pub(super) fn invalid_path(
        kind: &'static str,
        value: &str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self::invalid_input(format!("invalid {kind}: {error}"))
            .with_details(json!({"value": value}))
    }

    pub(super) fn path_not_found(path: &Path) -> Self {
        Self::not_found("working directory not found")
            .with_details(json!({"path": path.to_string_lossy()}))
    }
}

impl From<ApplicationError> for ApiError {
    fn from(error: ApplicationError) -> Self {
        let details = application_details(&error);
        let (status, code) = match (&error, error.code()) {
            (_, ApplicationErrorCode::InvalidInput) => {
                (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput)
            }
            (_, ApplicationErrorCode::NotFound) => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            (_, ApplicationErrorCode::Conflict) => (StatusCode::CONFLICT, ErrorCode::Conflict),
            (
                ApplicationError::Unavailable {
                    dependency: application::Dependency::Scheduler,
                    ..
                },
                ApplicationErrorCode::Unavailable,
            ) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::ServiceUnavailable,
            ),
            (_, ApplicationErrorCode::Timeout) => {
                (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Timeout)
            }
            (
                _,
                ApplicationErrorCode::Unavailable | ApplicationErrorCode::InvalidDependencyData,
            ) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InvalidDependencyData,
            ),
        };
        Self::new(status, code, error.to_string()).with_details(details)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: self.message.clone(),
            code: self.code,
            message: self.message,
            details: self.details,
        };
        (self.status, Json(body)).into_response()
    }
}

fn application_details(error: &ApplicationError) -> Value {
    match error {
        ApplicationError::JobNotFound { id } => json!({"job_id": id}),
        ApplicationError::SnapshotNotFound { path } => {
            json!({"path": path.to_string_lossy()})
        }
        ApplicationError::Conflict(Conflict::InvalidJobState {
            id,
            state,
            operation,
        }) => json!({"job_id": id, "state": state.as_str(), "operation": operation}),
        ApplicationError::Conflict(Conflict::StaleDescription {
            id,
            expected_revision,
            actual_revision,
        }) => json!({
            "job_id": id,
            "expected_revision": expected_revision,
            "actual_revision": actual_revision
        }),
        ApplicationError::Conflict(Conflict::QueueLocked) => json!({"queue_locked": true}),
        ApplicationError::Conflict(Conflict::QueueUnlocked) => json!({"queue_locked": false}),
        ApplicationError::Conflict(Conflict::StaleQueue) => json!({"stale": true}),
        ApplicationError::InvalidWorkingDirectory { path, .. } => {
            json!({"path": path.to_string_lossy()})
        }
        ApplicationError::Unavailable { dependency, .. }
        | ApplicationError::InvalidDependencyData { dependency, .. } => {
            json!({"dependency": dependency.to_string()})
        }
        ApplicationError::Timeout { operation } => json!({"operation": operation.to_string()}),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{Dependency, Operation};
    use crate::domain::JobState;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn application_errors_map_to_stable_status_codes_and_typed_details() {
        let id = Uuid::nil();
        let cases = [
            (
                ApplicationError::JobNotFound { id },
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
            ),
            (
                ApplicationError::Conflict(Conflict::StaleDescription {
                    id,
                    expected_revision: 1,
                    actual_revision: 2,
                }),
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
            ),
            (
                ApplicationError::Unavailable {
                    dependency: Dependency::Scheduler,
                    message: "offline".into(),
                },
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::ServiceUnavailable,
            ),
        ];
        for (error, status, code) in cases {
            let mapped = ApiError::from(error);
            assert_eq!(mapped.status, status);
            assert_eq!(mapped.code, code);
            assert_ne!(mapped.details, Value::Null);
        }
    }

    #[test]
    fn adapter_error_body_keeps_v1_and_typed_fields() {
        let body = ErrorResponse {
            error: "legacy".into(),
            code: ErrorCode::InvalidInput,
            message: "typed".into(),
            details: json!({"field": "name"}),
        };
        let value = serde_json::to_value(body).unwrap();
        for field in ["error", "code", "message", "details"] {
            assert!(value.get(field).is_some());
        }
    }

    #[test]
    fn every_application_error_category_maps_without_message_parsing() {
        let id = Uuid::nil();
        let cases = [
            (
                ApplicationError::InvalidCommand {
                    message: "empty".into(),
                },
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidInput,
            ),
            (
                ApplicationError::InvalidConfiguration {
                    message: "bad timezone".into(),
                },
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidInput,
            ),
            (
                ApplicationError::InvalidWorkingDirectory {
                    path: PathBuf::from("missing"),
                    message: "not found".into(),
                },
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidInput,
            ),
            (
                ApplicationError::SnapshotNotFound {
                    path: PathBuf::from("missing.toml"),
                },
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
            ),
            (
                ApplicationError::Conflict(Conflict::InvalidJobState {
                    id,
                    state: JobState::Running,
                    operation: "commit",
                }),
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
            ),
            (
                ApplicationError::Conflict(Conflict::QueueLocked),
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
            ),
            (
                ApplicationError::Conflict(Conflict::QueueUnlocked),
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
            ),
            (
                ApplicationError::Conflict(Conflict::StaleQueue),
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
            ),
            (
                ApplicationError::Unavailable {
                    dependency: Dependency::Configuration,
                    message: "offline".into(),
                },
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InvalidDependencyData,
            ),
            (
                ApplicationError::Timeout {
                    operation: Operation::Status,
                },
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Timeout,
            ),
            (
                ApplicationError::InvalidDependencyData {
                    dependency: Dependency::JobArtifacts,
                    message: "invalid bytes".into(),
                },
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InvalidDependencyData,
            ),
        ];

        for (error, status, code) in cases {
            let mapped = ApiError::from(error);
            assert_eq!(mapped.status, status);
            assert_eq!(mapped.code, code);
        }
    }

    #[test]
    fn adapter_constructors_produce_stable_status_code_and_details() {
        let cases = [
            ApiError::invalid_input("invalid"),
            ApiError::forbidden("origin"),
            ApiError::not_found("missing"),
            ApiError::method_not_allowed(),
            ApiError::internal("failure"),
            ApiError::invalid_path("path", "bad", "syntax"),
            ApiError::path_not_found(Path::new("missing")),
        ];
        let expected = [
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
            (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            (StatusCode::METHOD_NOT_ALLOWED, ErrorCode::MethodNotAllowed),
            (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
        ];
        for (actual, expected) in cases.into_iter().zip(expected) {
            assert_eq!((actual.status, actual.code), expected);
        }
    }
}
