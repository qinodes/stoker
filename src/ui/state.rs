use std::sync::Arc;

use tokio::sync::Notify;

use crate::Store;
use crate::adapters::LocalSchedulerGateway;
use crate::config::StokerPaths;

#[derive(Clone)]
pub(super) struct ApiState {
    pub(super) paths: StokerPaths,
    pub(super) store: Store,
    pub(super) scheduler: LocalSchedulerGateway,
    pub(super) shutdown: Arc<Notify>,
}

impl ApiState {
    pub(super) fn new(paths: StokerPaths, store: Store, scheduler: LocalSchedulerGateway) -> Self {
        Self {
            paths,
            store,
            scheduler,
            shutdown: Arc::new(Notify::new()),
        }
    }
}
