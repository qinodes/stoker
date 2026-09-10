use crate::application::Conflict;
use crate::application::model::{
    CommitSelection, DescriptionUpdate, JobFilter, PreparedJobInput, QueueMove, QueueSnapshot,
};
use crate::application::ports::{
    DescriptionUpdater, JobCanceller, JobCleaner, JobCommitter, JobCreator, JobQueries,
    JobRepositoryError, QueueRepository,
};
use crate::domain::{Job, NewJob};
use crate::{Store, StoreError};

impl JobQueries for Store {
    fn get_job(&self, id: uuid::Uuid) -> Result<Job, JobRepositoryError> {
        Store::get_job(self, id).map_err(map_store_error)
    }

    fn list_jobs(&self, filter: &JobFilter) -> Result<Vec<Job>, JobRepositoryError> {
        Store::list_jobs_with_state(self, filter.user.as_deref(), filter.state)
            .map_err(map_store_error)
    }
}

impl JobCreator for Store {
    fn create_job(&self, input: PreparedJobInput) -> Result<Job, JobRepositoryError> {
        let id = Store::create_shell_job(
            self,
            NewJob {
                name: input.name,
                user: input.user,
                description: input.description,
                cwd: input.cwd,
                command: input.command,
            },
            input.command_line,
        )
        .map_err(map_store_error)?;
        Store::get_job(self, id).map_err(map_store_error)
    }
}

impl DescriptionUpdater for Store {
    fn update_description(&self, input: DescriptionUpdate) -> Result<Job, JobRepositoryError> {
        Store::update_description(self, input.id, input.description, input.expected_revision)
            .map_err(map_store_error)
    }
}

impl JobCleaner for Store {
    fn clean_terminal_jobs(&self) -> Result<Vec<Job>, JobRepositoryError> {
        Store::clean_terminal_jobs(self).map_err(map_store_error)
    }
}

impl JobCommitter for Store {
    fn commit_jobs(&self, selection: &CommitSelection) -> Result<Vec<Job>, JobRepositoryError> {
        match selection {
            CommitSelection::Jobs(ids) => Store::commit_jobs(self, ids),
            CommitSelection::All => Store::commit_all_drafts(self),
            CommitSelection::User(user) => Store::commit_user_drafts(self, user),
        }
        .map_err(map_store_error)
    }
}

impl JobCanceller for Store {
    fn cancel_not_started(&self, id: uuid::Uuid) -> Result<Job, JobRepositoryError> {
        Store::cancel_not_started(self, id).map_err(map_store_error)
    }
}

impl QueueRepository for Store {
    fn queue_snapshot(&self) -> Result<QueueSnapshot, JobRepositoryError> {
        Ok(QueueSnapshot {
            jobs: Store::list_jobs_with_state(self, None, Some(crate::JobState::Queued))
                .map_err(map_store_error)?,
            locked: Store::queue_locked(self).map_err(map_store_error)?,
        })
    }

    fn set_queue_locked(&self, locked: bool) -> Result<QueueSnapshot, JobRepositoryError> {
        if locked {
            Store::lock_queue(self)
        } else {
            Store::unlock_queue(self)
        }
        .map_err(map_store_error)?;
        self.queue_snapshot()
    }

    fn move_queued(&self, movement: QueueMove) -> Result<QueueSnapshot, JobRepositoryError> {
        let jobs = Store::move_queued_job(self, movement.id, movement.target_order)
            .map_err(map_store_error)?;
        Ok(QueueSnapshot { jobs, locked: true })
    }
}

pub(super) fn map_store_error(error: StoreError) -> JobRepositoryError {
    match error {
        StoreError::NotFound { id } => JobRepositoryError::NotFound { id },
        StoreError::InvalidTransition { id, state, action } => {
            JobRepositoryError::Conflict(Conflict::InvalidJobState {
                id,
                state,
                operation: action,
            })
        }
        StoreError::QueueLocked => JobRepositoryError::Conflict(Conflict::QueueLocked),
        StoreError::QueueUnlocked => JobRepositoryError::Conflict(Conflict::QueueUnlocked),
        StoreError::InvalidQueueOrder { .. } => JobRepositoryError::Conflict(Conflict::StaleQueue),
        StoreError::DescriptionConflict {
            id,
            expected_revision,
            actual_revision,
        } => JobRepositoryError::Conflict(Conflict::StaleDescription {
            id,
            expected_revision,
            actual_revision,
        }),
        StoreError::InvalidData(message) => JobRepositoryError::InvalidData { message },
        StoreError::Serialization(error) => JobRepositoryError::InvalidData {
            message: error.to_string(),
        },
        StoreError::Database(error) => JobRepositoryError::Unavailable {
            message: error.to_string(),
        },
        StoreError::Poisoned => JobRepositoryError::Unavailable {
            message: "store lock is poisoned".to_owned(),
        },
    }
}
