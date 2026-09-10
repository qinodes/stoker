use std::sync::Arc;

use tokio::sync::Notify;

use crate::Store;
use crate::adapters::LocalSchedulerGateway;
use crate::config::StokerPaths;

use super::dto::UiMetadata;

#[derive(Clone)]
pub(super) struct ApiState {
    pub(super) paths: StokerPaths,
    pub(super) store: Store,
    pub(super) scheduler: LocalSchedulerGateway,
    pub(super) metadata: UiMetadata,
    pub(super) token: Option<String>,
    pub(super) shutdown: Arc<Notify>,
}

impl ApiState {
    pub(super) fn new(
        paths: StokerPaths,
        store: Store,
        scheduler: LocalSchedulerGateway,
        metadata: UiMetadata,
        token: Option<String>,
    ) -> Self {
        Self {
            paths,
            store,
            scheduler,
            metadata,
            token,
            shutdown: Arc::new(Notify::new()),
        }
    }
}
