//! Embedded browser UI façade.
//!
//! The implementation is split by HTTP adapter responsibility under `ui/`.

mod assets;
mod auth;
mod dto;
mod error;
mod handlers;
mod lifecycle;
mod router;
mod state;

pub use dto::UiMetadata;
pub use lifecycle::{run, start, status, stop};

pub const fn default_port() -> u16 {
    lifecycle::DEFAULT_PORT
}
