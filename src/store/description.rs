use rusqlite::{OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::{Job, normalize_description, validate_description};

use super::connection::Store;
use super::error::StoreError;
use super::mapping::get_job_with;

impl Store {
    /// Compare-and-swap prevents concurrent description updates from losing
    /// the winner's value.
    pub fn update_description(
        &self,
        id: Uuid,
        description: Option<String>,
        expected_revision: i64,
    ) -> Result<Job, StoreError> {
        let description = normalize_description(description);
        validate_description(description.as_deref()).map_err(StoreError::InvalidData)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "UPDATE jobs
             SET description = ?2, description_revision = description_revision + 1
             WHERE id = ?1 AND description_revision = ?3",
            params![id.to_string(), description, expected_revision],
        )?;
        if updated != 1 {
            let actual_revision: Option<i64> = transaction
                .query_row(
                    "SELECT description_revision FROM jobs WHERE id = ?1",
                    [id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            return match actual_revision {
                Some(actual_revision) => Err(StoreError::DescriptionConflict {
                    id,
                    expected_revision,
                    actual_revision,
                }),
                None => Err(StoreError::NotFound { id }),
            };
        }
        let job = get_job_with(&transaction, id)?;
        transaction.commit()?;
        Ok(job)
    }
}
