use chrono::{NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, TransactionBehavior, params};
use uuid::Uuid;

use crate::domain::{Job, JobState};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::fence_with;
use super::mapping::{get_job_with, parse_uuid};
use super::standalone::commit_standalone_definition;

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
        if fence_with(&transaction)? {
            let recovering: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM flow_runs WHERE state = 'RECOVERING'",
                [],
                |row| row.get(0),
            )?;
            if recovering > 0 {
                return Err(StoreError::InvalidData(
                    "queue recovery fence is active; reconcile recovery before unlocking".into(),
                ));
            }
            transaction.execute("UPDATE settings SET recovery_fence = 0 WHERE id = 1", [])?;
        }
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
        let now = Utc::now();
        let candidates = transaction
            .prepare(
                "SELECT id FROM jobs WHERE state = 'QUEUED'
                 AND mode = (SELECT mode FROM settings WHERE id = 1)
                 AND (
                     (mode = 'serial' AND retry = 0)
                     OR (mode = 'scheduled' AND NOT EXISTS (
                         SELECT 1 FROM flow_definitions f
                         WHERE f.flow_id = 'standalone/' || jobs.id
                     ))
                 )
                 ORDER BY queue_order, id",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut selected = None;
        for id_text in candidates {
            let id = parse_uuid(&id_text)?;
            if standalone_job_is_due(&transaction, id, now)? {
                selected = Some(id_text);
                break;
            }
        }
        let Some(id_text) = selected else {
            transaction.commit()?;
            return Ok(None);
        };
        let id = parse_uuid(&id_text)?;
        if transaction.execute(
            "UPDATE jobs SET state = 'STARTING', queue_order = NULL,
             last_dispatch_sequence = CASE WHEN schedule_kind = 'daily' THEN CAST(strftime('%Y%m%d', ?2) AS INTEGER) ELSE last_dispatch_sequence END
             WHERE id = ?1 AND state = 'QUEUED'",
            params![id.to_string(), now.to_rfc3339()],
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

fn standalone_job_is_due(
    connection: &Connection,
    id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<bool, StoreError> {
    let (mode, kind, at, daily, timezone, last_dispatch): (String, Option<String>, Option<String>, Option<String>, Option<String>, i64) = connection.query_row(
        "SELECT mode, schedule_kind, schedule_at_utc, daily_time, schedule_timezone, last_dispatch_sequence FROM jobs WHERE id = ?1",
        [id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    if mode == "serial" {
        return Ok(true);
    }
    match kind.as_deref() {
        Some("once") => Ok(at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .is_some_and(|value| {
                let at = value.with_timezone(&Utc);
                at <= now && at >= now - chrono::Duration::hours(24)
            })),
        Some("daily") => {
            let Some((daily, timezone)) = daily.zip(timezone) else {
                return Ok(false);
            };
            let zone: chrono_tz::Tz = timezone
                .parse()
                .map_err(|_| StoreError::InvalidData(format!("unknown timezone {timezone:?}")))?;
            let local = now.with_timezone(&zone);
            let date = local.date_naive();
            let wall_time = NaiveTime::parse_from_str(&daily, "%H:%M")
                .map_err(|error| StoreError::InvalidData(error.to_string()))?;
            let due = match zone.from_local_datetime(&date.and_time(wall_time)) {
                chrono::LocalResult::None => return Ok(false),
                chrono::LocalResult::Single(value) => value.with_timezone(&Utc),
                chrono::LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
            };
            let dispatch_key = date
                .format("%Y%m%d")
                .to_string()
                .parse::<i64>()
                .unwrap_or_default();
            Ok(last_dispatch != dispatch_key
                && due <= now
                && now <= due + chrono::Duration::seconds(5))
        }
        _ => Ok(false),
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
        commit_standalone_definition(
            connection,
            *id,
            first_order + i64::try_from(index).expect("queue length fits i64"),
            &committed_at,
        )?;
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

#[cfg(test)]
mod due_tests {
    use super::*;

    #[test]
    fn standalone_due_checks_once_daily_and_invalid_persisted_schedules() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("stoker.db")).unwrap();
        let id = store
            .create_job(crate::NewJob {
                name: "due fixture".into(),
                user: "tester".into(),
                description: None,
                cwd: directory.path().to_path_buf(),
                command: vec!["echo".into(), "due".into()],
            })
            .unwrap();
        let connection = store.lock().unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 8, 30, 2).unwrap();
        assert!(standalone_job_is_due(&connection, id, now).unwrap());

        connection.execute("UPDATE jobs SET mode = 'scheduled', schedule_kind = 'once', schedule_at_utc = '2026-09-23T08:30:00Z' WHERE id = ?1", [id.to_string()]).unwrap();
        assert!(standalone_job_is_due(&connection, id, now).unwrap());
        for at in ["2026-09-24T08:30:00Z", "2026-09-21T08:30:00Z", "not-a-date"] {
            connection
                .execute(
                    "UPDATE jobs SET schedule_at_utc = ?2 WHERE id = ?1",
                    params![id.to_string(), at],
                )
                .unwrap();
            assert!(
                !standalone_job_is_due(&connection, id, now).unwrap(),
                "{at}"
            );
        }

        connection.execute("UPDATE jobs SET schedule_kind = 'daily', daily_time = '08:30', schedule_timezone = 'UTC', last_dispatch_sequence = 0 WHERE id = ?1", [id.to_string()]).unwrap();
        assert!(standalone_job_is_due(&connection, id, now).unwrap());
        connection
            .execute(
                "UPDATE jobs SET last_dispatch_sequence = 20260923 WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(!standalone_job_is_due(&connection, id, now).unwrap());
        connection
            .execute(
                "UPDATE jobs SET last_dispatch_sequence = 0 WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(
            !standalone_job_is_due(&connection, id, now - chrono::Duration::seconds(3)).unwrap()
        );
        assert!(
            !standalone_job_is_due(&connection, id, now + chrono::Duration::seconds(4)).unwrap()
        );

        connection
            .execute(
                "UPDATE jobs SET schedule_timezone = NULL WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(!standalone_job_is_due(&connection, id, now).unwrap());
        connection
            .execute(
                "UPDATE jobs SET schedule_timezone = 'Not/A_Zone' WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(
            matches!(standalone_job_is_due(&connection, id, now), Err(StoreError::InvalidData(message)) if message.contains("timezone"))
        );
        connection
            .execute(
                "UPDATE jobs SET schedule_timezone = 'UTC', daily_time = 'bad-time' WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(matches!(
            standalone_job_is_due(&connection, id, now),
            Err(StoreError::InvalidData(_))
        ));

        connection.execute("UPDATE jobs SET schedule_timezone = 'America/New_York', daily_time = '02:30' WHERE id = ?1", [id.to_string()]).unwrap();
        let dst_gap = Utc.with_ymd_and_hms(2026, 3, 8, 7, 30, 0).unwrap();
        assert!(!standalone_job_is_due(&connection, id, dst_gap).unwrap());
        connection
            .execute(
                "UPDATE jobs SET schedule_kind = NULL WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(!standalone_job_is_due(&connection, id, now).unwrap());
    }
}
