//! SQLite store compatibility façade.
//!
//! Persistence ownership lives in focused modules under `store/`; the public
//! `stoker::store::{Store, StoreError}` paths remain stable.

mod connection;
mod description;
mod error;
mod jobs;
mod mapping;
mod migrations;
mod queue;
mod schema;
mod transition;

pub use connection::Store;
pub use error::StoreError;
pub use migrations::CURRENT_SCHEMA_VERSION;
