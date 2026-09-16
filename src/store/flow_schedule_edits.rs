//! Frozen-draft schedule editing and one-time terminal guards.

use chrono::Utc;
use rusqlite::TransactionBehavior;

use crate::domain::flow::{ExecutionMode, FlowDefinition, ScheduleSpec};

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{load_flow, load_flow_base, save_draft};
use super::flow_runtime_mapping::expire_occurrences;

impl Store {
    pub fn set_flow_schedule_draft(
        &self,
        flow_id: &str,
        schedule: ScheduleSpec,
        expected_draft_revision: Option<i64>,
    ) -> Result<FlowDefinition, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_occurrences(&transaction, Utc::now())?;
        let current = load_flow_base(&transaction, flow_id)?;
        if !current.frozen {
            return Err(StoreError::InvalidData(
                "flow must be frozen before editing its schedule".into(),
            ));
        }
        if current.mode != ExecutionMode::Scheduled {
            return Err(StoreError::InvalidData(
                "only scheduled flows have a schedule".into(),
            ));
        }
        if current
            .schedule
            .as_ref()
            .is_some_and(|current_schedule| matches!(current_schedule, ScheduleSpec::Once { .. }))
            != matches!(schedule, ScheduleSpec::Once { .. })
        {
            return Err(StoreError::InvalidData(
                "Task 003 does not support switching between once and daily schedules".into(),
            ));
        }
        if matches!(current.schedule, Some(ScheduleSpec::Once { .. }))
            && transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM occurrences WHERE flow_id = ?1 AND kind = 'once' AND state <> 'PENDING')",
                [flow_id],
                |row| row.get::<_, i64>(0),
            )? != 0
        {
            return Err(StoreError::InvalidData(
                "a terminal one-time definition cannot be rescheduled; create a new definition"
                    .into(),
            ));
        }
        if current.draft_revision > 0 && expected_draft_revision.is_none() {
            return Err(StoreError::InvalidData(
                "--expected-draft-revision is required when a draft exists".into(),
            ));
        }
        let expected = expected_draft_revision.unwrap_or(current.draft_revision);
        if expected != current.draft_revision {
            return Err(StoreError::InvalidData(format!(
                "draft revision conflict: expected {expected}, current {}",
                current.draft_revision
            )));
        }
        let mut draft = transaction
            .query_row(
                "SELECT draft_json FROM flow_definitions WHERE flow_id = ?1",
                [flow_id],
                |row| row.get::<_, Option<String>>(0),
            )?
            .map(|json| serde_json::from_str::<FlowDefinition>(&json))
            .transpose()?
            .unwrap_or_else(|| current.clone());
        if draft.schedule == Some(schedule.clone()) {
            return Ok(draft);
        }
        if let ScheduleSpec::Once { at } = schedule
            && at <= Utc::now()
        {
            return Err(StoreError::InvalidData(
                "new one-time schedule must be in the future".into(),
            ));
        }
        draft.schedule = Some(schedule);
        save_draft(&transaction, &draft, current.draft_revision + 1)?;
        let result = load_flow(&transaction, flow_id)?;
        transaction.commit()?;
        Ok(result)
    }
}
