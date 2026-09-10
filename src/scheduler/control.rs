//! Queue control commands.

use anyhow::Context;
use tokio::sync::watch;
use uuid::Uuid;

use crate::domain::Job;

use super::Scheduler;

impl Scheduler {
    pub fn handle_commit(&self, id: Uuid, wake: &watch::Sender<u64>) -> anyhow::Result<Job> {
        let job = self.store.commit_job(id).context("commit job")?;
        wake.send_modify(|value| *value = value.wrapping_add(1));
        Ok(job)
    }

    pub fn handle_commit_many(
        &self,
        ids: &[Uuid],
        wake: &watch::Sender<u64>,
    ) -> anyhow::Result<Vec<Job>> {
        let jobs = self.store.commit_jobs(ids).context("commit jobs")?;
        if !jobs.is_empty() {
            wake.send_modify(|value| *value = value.wrapping_add(1));
        }
        Ok(jobs)
    }

    pub fn handle_commit_all(&self, wake: &watch::Sender<u64>) -> anyhow::Result<Vec<Job>> {
        let jobs = self.store.commit_all_drafts().context("commit all jobs")?;
        if !jobs.is_empty() {
            wake.send_modify(|value| *value = value.wrapping_add(1));
        }
        Ok(jobs)
    }

    pub fn handle_commit_user(
        &self,
        user: &str,
        wake: &watch::Sender<u64>,
    ) -> anyhow::Result<Vec<Job>> {
        let jobs = self
            .store
            .commit_user_drafts(user)
            .context("commit user jobs")?;
        if !jobs.is_empty() {
            wake.send_modify(|value| *value = value.wrapping_add(1));
        }
        Ok(jobs)
    }

    pub fn handle_lock_queue(&self) -> anyhow::Result<()> {
        self.store.lock_queue().context("lock queue")?;
        Ok(())
    }

    pub fn handle_unlock_queue(&self, wake: &watch::Sender<u64>) -> anyhow::Result<()> {
        self.store.unlock_queue().context("unlock queue")?;
        wake.send_modify(|value| *value = value.wrapping_add(1));
        Ok(())
    }

    pub fn handle_move_queued(&self, id: Uuid, target_order: usize) -> anyhow::Result<Vec<Job>> {
        self.store
            .move_queued_job(id, target_order)
            .map_err(anyhow::Error::from)
    }
}
