//! Scheduler status, queue snapshot, and log access.

use std::path::PathBuf;

use anyhow::Context;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::domain::{Job, JobState};

use super::{LogMessage, Scheduler, SchedulerStatus};

impl Scheduler {
    #[cfg(test)]
    pub(crate) fn install_test_log_sender(
        &self,
        id: Uuid,
        capacity: usize,
    ) -> broadcast::Sender<LogMessage> {
        let (sender, _) = broadcast::channel(capacity);
        self.logs.lock().unwrap().insert(id, sender.clone());
        sender
    }

    /// Return the scheduler-owned status model without exposing transport DTOs.
    pub fn scheduler_status(&self) -> anyhow::Result<SchedulerStatus> {
        let jobs = self.store.list_jobs(None)?;
        let queue_locked = self.store.queue_locked()?;
        let active_job = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler active slot is poisoned"))?
            .as_ref()
            .map(|active| active.id)
            .or_else(|| {
                jobs.iter()
                    .find(|job| {
                        matches!(
                            job.state,
                            JobState::Starting | JobState::Running | JobState::Cancelling
                        )
                    })
                    .map(|job| job.id)
            });
        Ok(SchedulerStatus {
            pid: std::process::id(),
            active_job,
            queued_jobs: jobs
                .iter()
                .filter(|job| job.state == JobState::Queued)
                .count(),
            queue_locked,
        })
    }

    pub(crate) fn queue_snapshot(&self) -> anyhow::Result<(Vec<Job>, bool)> {
        let jobs = self
            .store
            .list_jobs(None)
            .context("list queued jobs")?
            .into_iter()
            .filter(|job| job.state == JobState::Queued)
            .collect();
        let locked = self.store.queue_locked().context("read queue lock")?;
        Ok((jobs, locked))
    }

    pub(crate) fn log_paths(&self, id: Uuid) -> (PathBuf, PathBuf) {
        let run = self.paths.runs.join(id.to_string());
        (run.join("stdout.log"), run.join("stderr.log"))
    }

    pub(crate) fn log_receiver(&self, id: Uuid) -> Option<broadcast::Receiver<LogMessage>> {
        self.logs
            .lock()
            .ok()
            .and_then(|logs| logs.get(&id).map(broadcast::Sender::subscribe))
    }

    pub(crate) fn job_exists(&self, id: Uuid) -> anyhow::Result<Job> {
        Ok(self.store.get_job(id)?)
    }
}
