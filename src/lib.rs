pub(crate) mod adapters;
pub mod application;
pub mod cli;
pub mod config;
pub mod domain;
pub mod ipc;
pub(crate) mod log_storage;
pub mod output;
pub mod process;
pub(crate) mod queue_editor;
pub mod scheduler;
mod scheduler_compatibility;
pub mod service;
pub mod store;
pub mod submission;
pub mod ui;

pub use config::StokerPaths;
pub use domain::{
    DomainError, DomainErrorCode, Job, JobState, MAX_JOB_DESCRIPTION_LENGTH, MAX_JOB_NAME_LENGTH,
    MAX_JOB_USER_LENGTH, NewJob, ValidationField, normalize_description, validate_description,
    validate_job_name, validate_job_user,
};
pub use ipc::{
    ClientLogEvent, ClientLogStream, IPC_VERSION, InvalidServiceResponse, IpcError, IpcErrorCode,
    IpcErrorDetails, IpcRequest, IpcResponse, JobDto, JobStateDto, ProtocolVersionMismatch,
    ServiceClient, ServiceRejected, ServiceStatus, ServiceTimeout, ServiceUnavailable,
    StaleQueueMoveError, is_service_unavailable,
};
pub use store::{Store, StoreError};
