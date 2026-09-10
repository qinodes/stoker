//! Queue use cases and the single scheduler-offline fallback policy.

use crate::application::error::ApplicationResult;
use crate::application::model::{QueueLockResult, QueueMove, QueueStatus};
use crate::application::ports::{
    QueueRepository, SchedulerGatewayError, SchedulerQueueGateway, SchedulerStatusGateway,
};

pub fn queue_snapshot(
    repository: &impl QueueRepository,
) -> ApplicationResult<crate::application::QueueSnapshot> {
    repository.queue_snapshot().map_err(Into::into)
}

pub async fn queue_status(
    repository: &impl QueueRepository,
    scheduler: &impl SchedulerStatusGateway,
) -> ApplicationResult<QueueStatus> {
    match scheduler.scheduler_status().await {
        Ok(status) => {
            let mut snapshot = queue_snapshot(repository)?;
            snapshot.locked = status.queue_locked;
            Ok(QueueStatus {
                snapshot,
                scheduler: Some(status),
            })
        }
        Err(SchedulerGatewayError::Unavailable { .. }) => Ok(QueueStatus {
            snapshot: queue_snapshot(repository)?,
            scheduler: None,
        }),
        Err(error) => Err(error.into()),
    }
}

pub async fn set_queue_locked(
    repository: &impl QueueRepository,
    scheduler: &(impl SchedulerStatusGateway + SchedulerQueueGateway),
    locked: bool,
) -> ApplicationResult<QueueLockResult> {
    let before = queue_status(repository, scheduler).await?;
    let scheduler_online = if before.scheduler_online() {
        match scheduler.set_queue_locked(locked).await {
            Ok(_) => true,
            Err(SchedulerGatewayError::Unavailable { .. }) => {
                repository.set_queue_locked(locked)?;
                false
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        repository.set_queue_locked(locked)?;
        false
    };
    let mut snapshot = queue_snapshot(repository)?;
    snapshot.locked = locked;
    let scheduler = if scheduler_online {
        before.scheduler.clone()
    } else {
        None
    };
    Ok(QueueLockResult {
        before,
        after: QueueStatus {
            snapshot,
            scheduler,
        },
    })
}

pub async fn move_queued(
    repository: &impl QueueRepository,
    scheduler: &(impl SchedulerStatusGateway + SchedulerQueueGateway),
    movement: QueueMove,
) -> ApplicationResult<QueueStatus> {
    let status = queue_status(repository, scheduler).await?;
    if status.scheduler_online() {
        match scheduler.move_queued(movement).await {
            Ok(snapshot) => Ok(QueueStatus {
                snapshot,
                scheduler: status.scheduler,
            }),
            Err(SchedulerGatewayError::Unavailable { .. }) => Ok(QueueStatus {
                snapshot: repository.move_queued(movement)?,
                scheduler: None,
            }),
            Err(error) => Err(error.into()),
        }
    } else {
        Ok(QueueStatus {
            snapshot: repository.move_queued(movement)?,
            scheduler: None,
        })
    }
}
