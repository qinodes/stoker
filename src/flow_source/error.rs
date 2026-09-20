use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlowSourceError {
    #[error("invalid Flow definition document: {0}")]
    Invalid(String),
    #[error("Flow definition JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not access Flow definition path {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl FlowSourceError {
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub(super) fn filesystem(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Filesystem {
            path: path.into(),
            source,
        }
    }
}
