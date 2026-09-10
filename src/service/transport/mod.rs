//! Platform-specific service listeners.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::sync::Arc;

use tokio::sync::watch;

use crate::scheduler::Scheduler;

use super::Service;

pub(super) async fn run(
    service: Service,
    scheduler: Arc<Scheduler>,
    wake_tx: watch::Sender<u64>,
    wake_rx: watch::Receiver<u64>,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    return unix::run(service, scheduler, wake_tx, wake_rx).await;

    #[cfg(windows)]
    return windows::run(service, scheduler, wake_tx, wake_rx).await;
}
