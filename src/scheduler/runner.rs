//! Single-slot queue runner.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::watch;

use crate::log_storage;

use super::{ActiveExecution, Scheduler};

impl Scheduler {
    pub async fn run(
        self: Arc<Self>,
        mut wake: watch::Receiver<u64>,
        mut shutdown: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        let mut flow_tasks = tokio::task::JoinSet::new();
        loop {
            if (*shutdown.borrow() || self.stopping.load(Ordering::Acquire))
                && flow_tasks.is_empty()
            {
                return Ok(());
            }
            let policy = self.store.log_policy()?;
            log_storage::enforce_retention(&self.paths, &self.store, policy)?;
            if log_storage::available_space(&self.paths.runs)? < policy.disk_reserve_bytes {
                tokio::select! {
                    changed = wake.changed() => {
                        if changed.is_err() { return Ok(()); }
                    }
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
                continue;
            }
            while let Some(result) = flow_tasks.try_join_next() {
                result.context("scheduled flow task panicked")??;
            }
            if *shutdown.borrow() || self.stopping.load(Ordering::Acquire) {
                if let Some(result) = flow_tasks.join_next().await {
                    result.context("scheduled flow task panicked")??;
                }
                continue;
            }
            let mode = self.store.current_mode()?;
            if mode == crate::domain::flow::ExecutionMode::Scheduled {
                let concurrency = self.store.scheduled_concurrency()? as usize;
                while flow_tasks.len() < concurrency {
                    let Some(flow_task) = self.store.claim_flow_task(chrono::Utc::now())? else {
                        break;
                    };
                    let scheduler = Arc::clone(&self);
                    flow_tasks.spawn(async move { scheduler.execute_flow_task(flow_task).await });
                }
                self.store.settle_recurring_occurrences(
                    chrono::Utc::now(),
                    flow_tasks.len() >= concurrency,
                )?;
                if !flow_tasks.is_empty() {
                    tokio::select! {
                        result = flow_tasks.join_next() => {
                            if let Some(result) = result {
                                result.context("scheduled flow task panicked")??;
                            }
                        }
                        changed = wake.changed() => {
                            if changed.is_err() { return Ok(()); }
                        }
                        changed = shutdown.changed() => {
                            if changed.is_err() { return Ok(()); }
                        }
                    }
                    continue;
                }
            } else if let Some(flow_task) = self.store.claim_flow_task(chrono::Utc::now())? {
                self.execute_flow_task(flow_task).await?;
                continue;
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
            if !flow_tasks.is_empty() {
                continue;
            }
            tokio::select! {
                changed = wake.changed() => {
                    if changed.is_err() { return Ok(()); }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }
}
