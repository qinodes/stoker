//! Narrow application-owned ports implemented by outer adapters.

pub mod clock;
pub mod configuration;
pub mod jobs;
pub mod scheduler;

pub use clock::Clock;
pub use configuration::{
    ConfigurationReader, ConfigurationRepositoryError, ConfigurationSnapshots, ConfigurationWriter,
};
pub use jobs::{
    DescriptionUpdater, JobArtifacts, JobArtifactsError, JobCanceller, JobCleaner, JobCommitter,
    JobCreator, JobQueries, JobRepositoryError, QueueRepository, WorkingDirectoryResolver,
};
pub use scheduler::{
    LogEventStream, SchedulerCancelGateway, SchedulerCommitGateway, SchedulerGatewayError,
    SchedulerLogGateway, SchedulerQueueGateway, SchedulerStatusGateway,
};
