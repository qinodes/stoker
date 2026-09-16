//! Active and queued job cancellation.

use std::sync::atomic::Ordering;

use anyhow::Context;
use uuid::Uuid;

use crate::domain::{Job, JobState};

use super::Scheduler;

impl Scheduler {
    /// Cancel a job, waiting for the active process and runtime cleanup before
    /// acknowledging an active cancellation request.
    pub async fn handle_cancel(&self, id: Uuid) -> anyhow::Result<Job> {
        let job = self
            .store
            .get_job(id)
            .context("inspect job for cancellation")?;
        match job.state {
            JobState::Draft | JobState::Queued => {
                Ok(self.store.cancel_not_started(id).context("cancel job")?)
            }
            JobState::Starting | JobState::Running | JobState::Cancelling => {
                self.cancel_active(id).await
            }
            state => anyhow::bail!("cannot cancel job {id} while it is {state}"),
        }
    }

    /// Request shutdown cancellation for the current execution and wait until
    /// its terminal state and cleanup have been persisted.
    pub async fn stop_active(&self) -> anyhow::Result<()> {
        self.store
            .cancel_all_flow_runs()
            .context("cancel active flow runs")?;
        let id = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler active slot is poisoned"))?
            .as_ref()
            .map(|active| active.id);
        if let Some(id) = id {
            self.cancel_active(id).await?;
        }
        Ok(())
    }

    /// Close the scheduler intake. This is set before the service shutdown
    /// watch is broadcast so a concurrent queue wake cannot claim another job
    /// while Stop is waiting for active cleanup.
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    pub(super) async fn cancel_active(&self, id: Uuid) -> anyhow::Result<Job> {
        let (active, completed) = {
            let active = self
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("scheduler active slot is poisoned"))?;
            let active = active.as_ref().filter(|active| active.id == id).cloned();
            let completed = active.as_ref().map(|active| active.completed.subscribe());
            (active, completed)
        };
        let Some(active) = active else {
            let current = self.store.get_job(id).context("inspect active job")?;
            if matches!(
                current.state,
                JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
            ) {
                anyhow::bail!("cannot cancel job {id} while it is {}", current.state);
            }
            anyhow::bail!("job {id} is not managed by the active scheduler");
        };
        let mut completed = completed.expect("active completion receiver must exist");
        let current = self.store.get_job(id).context("inspect active job")?;
        if matches!(
            current.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
        ) {
            while !*completed.borrow() {
                completed
                    .changed()
                    .await
                    .with_context(|| format!("wait for completed job {id}"))?;
            }
            return self.store.get_job(id).context("inspect completed job");
        }
        if current.state != JobState::Cancelling
            && let Err(error) = self.store.request_cancelling(id)
        {
            // The process may finish between the state check above and the
            // transition. Treat that terminal race, or an already requested
            // cancellation, as a successful stop.
            let current = self.store.get_job(id).context("inspect active job")?;
            if !matches!(
                current.state,
                JobState::Succeeded
                    | JobState::Failed
                    | JobState::Cancelled
                    | JobState::Lost
                    | JobState::Cancelling
            ) {
                return Err(error).context(format!(
                    "mark job cancelling while job is {}",
                    current.state
                ));
            }
            if current.state == JobState::Cancelling {
                let _ = active.cancel.send(true);
            }
            while !*completed.borrow() {
                completed
                    .changed()
                    .await
                    .with_context(|| format!("wait for completed job {id}"))?;
            }
            return self.store.get_job(id).context("inspect completed job");
        }
        let _ = active.cancel.send(true);
        loop {
            let current = self.store.get_job(id).context("inspect cancelled job")?;
            if *completed.borrow()
                && matches!(
                    current.state,
                    JobState::Succeeded | JobState::Failed | JobState::Cancelled | JobState::Lost
                )
            {
                return Ok(current);
            }
            // `watch` retains the completion value, so a completion racing
            // with this check cannot be lost between polling and awaiting.
            if completed.changed().await.is_err() {
                anyhow::bail!("scheduler completion signal closed before cleanup for job {id}");
            }
        }
    }
}
