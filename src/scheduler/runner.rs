//! Single-slot queue runner.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::watch;

use super::{ActiveExecution, Scheduler};

impl Scheduler {
    pub async fn run(
        self: Arc<Self>,
        mut wake: watch::Receiver<u64>,
        mut shutdown: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        loop {
            if *shutdown.borrow() || self.stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(job) = self.store.claim_next()? {
                let (cancel, cancel_rx) = watch::channel(false);
                let (completed, _) = watch::channel(false);
                if let Ok(mut active) = self.active.lock() {
                    *active = Some(ActiveExecution {
                        id: job.id,
                        cancel,
                        completed: completed.clone(),
                    });
                }
                let result = self.execute(job, cancel_rx).await;
                if let Ok(mut active) = self.active.lock() {
                    *active = None;
                }
                let _ = completed.send(true);
                // An unrecoverable runtime cleanup error keeps the slot
                // occupied and stops queue progression.
                result?;
                continue;
            }
            tokio::select! {
                changed = wake.changed() => {
                    if changed.is_err() { return Ok(()); }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
            }
        }
    }
}
