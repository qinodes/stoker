use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::{Job, JobState};

use super::connection::Store;
use super::error::StoreError;
use super::mapping::{get_job_with, parse_uuid};

const COMMIT_DRAFT_SQL: &str =
    "UPDATE jobs SET state = 'QUEUED', queue_order = ?2, committed_at = ?3
     WHERE id = ?1 AND state = 'DRAFT'";

impl Store {
    pub fn queue_locked(&self) -> Result<bool, StoreError> {
        let connection = self.lock()?;
        queue_locked_with(&connection)
    }

    pub fn lock_queue(&self) -> Result<(), StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("UPDATE settings SET queue_locked = 1 WHERE id = 1", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn unlock_queue(&self) -> Result<(), StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("UPDATE settings SET queue_locked = 0 WHERE id = 1", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn commit_job(&self, id: Uuid) -> Result<Job, StoreError> {
        self.commit_jobs(&[id])?
            .into_iter()
            .next()
            .ok_or(StoreError::NotFound { id })
    }

    pub fn commit_jobs(&self, ids: &[Uuid]) -> Result<Vec<Job>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_queue_unlocked(&transaction)?;
        for id in ids {
            let state = self.current_state(&transaction, *id)?;
            if state != JobState::Draft {
                return Err(StoreError::InvalidTransition {
                    id: *id,
                    state,
                    action: "commit",
                });
            }
        }
        let jobs = commit_draft_ids(&transaction, ids)?;
        transaction.commit()?;
        Ok(jobs)
    }

    pub fn commit_all_drafts(&self) -> Result<Vec<Job>, StoreError> {
        self.commit_drafts_for_user(None)
    }

    pub fn commit_user_drafts(&self, user: &str) -> Result<Vec<Job>, StoreError> {
        self.commit_drafts_for_user(Some(user))
    }

    fn commit_drafts_for_user(&self, user: Option<&str>) -> Result<Vec<Job>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_queue_unlocked(&transaction)?;
        let ids = draft_ids(&transaction, user)?;
        let jobs = commit_draft_ids(&transaction, &ids)?;
        transaction.commit()?;
        Ok(jobs)
    }

    pub fn cancel_not_started(&self, id: Uuid) -> Result<Job, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = self.current_state(&transaction, id)?;
        if !matches!(state, JobState::Draft | JobState::Queued) {
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "cancel",
            });
        }
        if transaction.execute(
            "UPDATE jobs SET state = 'CANCELLED', queue_order = NULL, finished_at = ?2
             WHERE id = ?1 AND state IN ('DRAFT', 'QUEUED')",
            params![id.to_string(), Utc::now().to_rfc3339()],
        )? != 1
        {
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "cancel",
            });
        }
        if state == JobState::Queued {
            normalize_queue(&transaction)?;
        }
        let job = get_job_with(&transaction, id)?;
        transaction.commit()?;
        Ok(job)
    }

    pub fn move_queued_job(&self, id: Uuid, target_order: usize) -> Result<Vec<Job>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !queue_locked_with(&transaction)? {
            return Err(StoreError::QueueUnlocked);
        }

        let mut queued_ids = transaction
            .prepare(
                "SELECT id FROM jobs WHERE state = 'QUEUED'
                 ORDER BY queue_order, id",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|id_text| parse_uuid(&id_text))
            .collect::<Result<Vec<_>, _>>()?;
        let queued_count = queued_ids.len();
        if !queued_ids.contains(&id) {
            let state = self.current_state(&transaction, id)?;
            return Err(StoreError::InvalidTransition {
                id,
                state,
                action: "move",
            });
        }
        if !(1..=queued_count).contains(&target_order) {
            return Err(StoreError::InvalidQueueOrder {
                id,
                target_order,
                queued_count,
            });
        }

        let selected_index = queued_ids
            .iter()
            .position(|queued_id| *queued_id == id)
            .expect("selected queued job was checked above");
        let selected = queued_ids.remove(selected_index);
        queued_ids.insert(target_order - 1, selected);
        for (index, queued_id) in queued_ids.iter().enumerate() {
            transaction.execute(
                "UPDATE jobs SET queue_order = ?2 WHERE id = ?1 AND state = 'QUEUED'",
                params![
                    queued_id.to_string(),
                    i64::try_from(index + 1).expect("queue length fits i64")
                ],
            )?;
        }
        let jobs = queued_ids
            .into_iter()
            .map(|queued_id| get_job_with(&transaction, queued_id))
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit()?;
        Ok(jobs)
    }

    /// Atomically claim the oldest queued job. SQLite's write transaction
    /// prevents concurrent scheduler instances from selecting the same job.
    pub fn claim_next(&self) -> Result<Option<Job>, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if queue_locked_with(&transaction)? {
            transaction.commit()?;
            return Ok(None);
        }
        let selected: Option<String> = transaction
            .query_row(
                "SELECT id FROM jobs WHERE state = 'QUEUED'
                 ORDER BY queue_order, id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id_text) = selected else {
            transaction.commit()?;
            return Ok(None);
        };
        let id = parse_uuid(&id_text)?;
        if transaction.execute(
            "UPDATE jobs SET state = 'STARTING', queue_order = NULL
             WHERE id = ?1 AND state = 'QUEUED'",
            [id.to_string()],
        )? != 1
        {
            return Err(StoreError::InvalidTransition {
                id,
                state: JobState::Queued,
                action: "claim",
            });
        }
        normalize_queue(&transaction)?;
        let job = get_job_with(&transaction, id)?;
        transaction.commit()?;
        Ok(Some(job))
    }
}

fn queue_locked_with(connection: &Connection) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT queue_locked FROM settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

fn ensure_queue_unlocked(connection: &Connection) -> Result<(), StoreError> {
    if queue_locked_with(connection)? {
        Err(StoreError::QueueLocked)
    } else {
        Ok(())
    }
}

fn draft_ids(connection: &Connection, user: Option<&str>) -> Result<Vec<Uuid>, StoreError> {
    let ids = {
        let query = match user {
            Some(_) => {
                "SELECT id FROM jobs WHERE user = ?1 AND state = 'DRAFT' ORDER BY created_at, id"
            }
            None => "SELECT id FROM jobs WHERE state = 'DRAFT' ORDER BY created_at, id",
        };
        let mut statement = connection.prepare(query)?;
        match user {
            Some(user) => statement
                .query_map([user], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
            None => statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        }
    };
    ids.into_iter().map(|id| parse_uuid(&id)).collect()
}

fn commit_draft_ids(connection: &Connection, ids: &[Uuid]) -> Result<Vec<Job>, StoreError> {
    let first_order = next_queue_order(connection)?;
    let committed_at = Utc::now().to_rfc3339();
    for (index, id) in ids.iter().enumerate() {
        let updated = connection.execute(
            COMMIT_DRAFT_SQL,
            params![
                id.to_string(),
                first_order + i64::try_from(index).expect("queue length fits i64"),
                committed_at,
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::InvalidTransition {
                id: *id,
                state: JobState::Draft,
                action: "commit",
            });
        }
    }
    if !ids.is_empty() {
        normalize_queue(connection)?;
    }
    ids.iter()
        .copied()
        .map(|id| get_job_with(connection, id))
        .collect()
}

fn next_queue_order(connection: &Connection) -> Result<i64, StoreError> {
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM jobs WHERE state = 'QUEUED'",
        [],
        |row| row.get(0),
    )?)
}

fn normalize_queue(connection: &Connection) -> Result<(), StoreError> {
    let ids = connection
        .prepare(
            "SELECT id FROM jobs WHERE state = 'QUEUED'
             ORDER BY queue_order, id",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for (index, id) in ids.iter().enumerate() {
        connection.execute(
            "UPDATE jobs SET queue_order = ?2 WHERE id = ?1",
            params![id, i64::try_from(index + 1).expect("queue length fits i64")],
        )?;
    }
    Ok(())
}
