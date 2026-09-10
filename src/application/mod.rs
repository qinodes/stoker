//! Framework-neutral application contracts.
//!
//! Adapters implement the narrow ports in [`ports`]. Use cases added in later
//! stages depend on these contracts, never on Clap, HTTP, IPC, SQLite or Tokio.

pub mod configuration;
pub mod error;
pub mod jobs;
pub mod logs;
pub mod model;
pub mod ports;
pub mod queue;

pub use error::{
    ApplicationError, ApplicationErrorCode, ApplicationResult, Conflict, Dependency, Operation,
};
pub use model::{
    ApplicationConfig, CommitSelection, ConfigSnapshot, CreateJobInput, DescriptionUpdate,
    JobFilter, JobLogs, LogContent, LogEvent, OutputStream, PreparedJobInput, QueueLockResult,
    QueueMove, QueueSnapshot, QueueStatus, SchedulerStatus, SnapshotReason,
};
