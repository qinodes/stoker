//! Typed client-side IPC failures.

use crate::ipc::protocol::IpcError;

#[derive(Debug, thiserror::Error)]
#[error("scheduler service unavailable: {0}")]
pub struct ServiceUnavailable(#[source] pub(crate) std::io::Error);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{error_message}")]
pub struct ServiceRejected {
    pub error: IpcError,
    error_message: String,
}

impl ServiceRejected {
    pub(crate) fn new(error: IpcError) -> Self {
        Self {
            error_message: error.message.clone(),
            error,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{operation} timed out")]
pub struct ServiceTimeout {
    pub operation: &'static str,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("unsupported IPC protocol version {actual}; expected {expected}")]
pub struct ProtocolVersionMismatch {
    pub expected: u16,
    pub actual: u16,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("service returned an invalid {operation} response: {response}")]
pub struct InvalidServiceResponse {
    pub operation: &'static str,
    pub response: &'static str,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct StaleQueueMoveError {
    message: String,
}

impl StaleQueueMoveError {
    pub(crate) fn new(message: String) -> Self {
        Self { message }
    }
}

pub fn is_service_unavailable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ServiceUnavailable>().is_some()
}
