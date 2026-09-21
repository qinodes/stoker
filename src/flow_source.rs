//! Versioned, portable Flow definition documents.
//!
//! The format model and canonical hashing are independent from SQLite and the
//! CLI. Filesystem concerns are isolated in focused modules so sync can reuse
//! the same validated representation in later stages.

mod codec;
mod error;
mod filesystem;
mod model;
mod resolution;

pub use codec::{canonical_hash, format_document, parse_document};
pub use error::FlowSourceError;
pub use filesystem::{archive_source, verify_artifact, write_export, write_snapshot};
pub use model::{
    FLOW_SOURCE_SCHEMA_VERSION, FlowSourceBase, FlowSourceCwd, FlowSourceCwdMap,
    FlowSourceDependency, FlowSourceDocument, FlowSourceFlow, FlowSourceSchedule, FlowSourceTask,
};
pub use resolution::{resolve_document, resolve_workspace_document};
