use thiserror::Error;
use uuid::Uuid;

use crate::domain::JobState;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("job {id} does not exist")]
    NotFound { id: Uuid },
    #[error("cannot {action} job {id} while it is {state}")]
    InvalidTransition {
        id: Uuid,
        state: JobState,
        action: &'static str,
    },
    #[error("queue is locked; run 'stoker queue unlock'")]
    QueueLocked,
    #[error("queue is unlocked; run 'stoker queue lock' first")]
    QueueUnlocked,
    #[error("cannot move job {id} to queue position {target_order}; queue has {queued_count} jobs")]
    InvalidQueueOrder {
        id: Uuid,
        target_order: usize,
        queued_count: usize,
    },
    #[error(
        "description for job {id} changed concurrently (expected revision {expected_revision}, current revision {actual_revision})"
    )]
    DescriptionConflict {
        id: Uuid,
        expected_revision: i64,
        actual_revision: i64,
    },
    #[error("store lock is poisoned")]
    Poisoned,
    #[error("invalid value in jobs table: {0}")]
    InvalidData(String),
}
