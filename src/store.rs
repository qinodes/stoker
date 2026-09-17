//! SQLite store compatibility façade.
//!
//! Persistence ownership lives in focused modules under `store/`; the public
//! `stoker::store::{Store, StoreError}` paths remain stable.

mod connection;
mod description;
mod error;
mod flow_cancellation;
mod flow_edits;
mod flow_mapping;
mod flow_policy;
mod flow_run_dispatch;
mod flow_runtime;
mod flow_runtime_mapping;
mod flow_schedule_edits;
mod flow_schedule_mapping;
mod flows;
mod health;
mod jobs;
mod log_policy;
mod mapping;
mod migrations;
mod queue;
mod runtime_policy;
mod schema;
mod standalone;
mod transition;

pub use connection::Store;
pub use error::StoreError;
pub use flow_edits::ManualRequestStatus;
pub use flows::{FlowAttemptResult, FlowTaskExecution, FlowTaskInput, StandaloneDefinition};
pub use migrations::CURRENT_SCHEMA_VERSION;
