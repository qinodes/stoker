//! Source-compatible scheduler adapters kept outside the scheduler core.

use crate::ipc::ServiceStatus;
use crate::scheduler::Scheduler;

impl Scheduler {
    /// Return the legacy IPC-shaped service status.
    ///
    /// New integrations should use [`Self::scheduler_status`] and map the
    /// scheduler-owned model at their transport boundary.
    #[deprecated(note = "use scheduler_status for the scheduler-owned model")]
    pub fn service_status(&self) -> anyhow::Result<ServiceStatus> {
        self.scheduler_status().map(Into::into)
    }
}
