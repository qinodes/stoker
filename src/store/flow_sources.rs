//! Declarative Flow workspace state and transactional source-mode gates.

use std::fmt;
use std::str::FromStr;

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::domain::flow::{ExecutionMode, FlowDefinition};
use crate::flow_source::FlowSourceDocument;

use super::connection::Store;
use super::error::StoreError;
use super::flow_mapping::{has_active_work, load_flow_current, parse_mode, require_queue_locked};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowSourceMode {
    Manual,
    Sync,
}

impl fmt::Display for FlowSourceMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Manual => "manual",
            Self::Sync => "sync",
        })
    }
}

impl FromStr for FlowSourceMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manual" => Ok(Self::Manual),
            "sync" => Ok(Self::Sync),
            _ => Err("source mode must be manual or sync".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowSourceState {
    pub mode: FlowSourceMode,
    pub revision: i64,
    pub hash: String,
}

impl Store {
    pub fn export_flow_source(&self) -> Result<FlowSourceDocument, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (_, revision, _) = source_state_with(&transaction)?;
        let document = FlowSourceDocument::from_definitions(
            revision,
            &load_committed_definitions(&transaction)?,
        )
        .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        transaction.commit()?;
        Ok(document)
    }

    pub fn flow_source_state(&self) -> Result<FlowSourceState, StoreError> {
        let connection = self.lock()?;
        let (mode, revision, stored_hash) = source_state_with(&connection)?;
        let actual = hash_committed_definitions(&connection, revision)?;
        Ok(FlowSourceState {
            mode,
            revision,
            hash: if stored_hash.is_empty() {
                actual
            } else {
                stored_hash
            },
        })
    }

    pub fn set_flow_source_mode(
        &self,
        requested: FlowSourceMode,
    ) -> Result<FlowSourceState, StoreError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let execution: String =
            transaction.query_row("SELECT mode FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        if parse_mode(&execution)? != ExecutionMode::Scheduled {
            return Err(StoreError::InvalidData(
                "flow source mode is only available in scheduled execution mode".into(),
            ));
        }
        require_queue_locked(&transaction)?;
        if has_active_work(&transaction)? {
            return Err(StoreError::InvalidData(
                "cannot change flow source mode while work is active or recovery is incomplete"
                    .into(),
            ));
        }
        if requested == FlowSourceMode::Sync {
            reject_unready_definitions(&transaction)?;
        }
        let (_, revision, _) = source_state_with(&transaction)?;
        let hash = hash_committed_definitions(&transaction, revision)?;
        transaction.execute(
            "UPDATE settings SET flow_source_mode = ?1, flow_definition_hash = ?2 WHERE id = 1",
            params![requested.to_string(), hash],
        )?;
        transaction.commit()?;
        Ok(FlowSourceState {
            mode: requested,
            revision,
            hash,
        })
    }
}

pub(super) fn require_manual_source(
    connection: &Connection,
    flow_id: Option<&str>,
) -> Result<(), StoreError> {
    if flow_id.is_some_and(|id| id.starts_with("standalone/")) {
        return Ok(());
    }
    let (mode, _, _) = source_state_with(connection)?;
    if mode != FlowSourceMode::Manual {
        return Err(StoreError::InvalidData("Flow definitions are managed by sync source mode; edit the JSON definition and run `stoker flow sync`, or switch to manual mode while the queue is locked".into()));
    }
    Ok(())
}

pub(super) fn advance_definition_state(
    transaction: &Transaction<'_>,
) -> Result<FlowSourceState, StoreError> {
    let (mode, revision, _) = source_state_with(transaction)?;
    let next = revision
        .checked_add(1)
        .ok_or_else(|| StoreError::InvalidData("definition revision overflow".into()))?;
    let hash = hash_committed_definitions(transaction, next)?;
    transaction.execute(
        "UPDATE settings SET flow_definition_revision = ?1, flow_definition_hash = ?2 WHERE id = 1",
        params![next, hash],
    )?;
    Ok(FlowSourceState {
        mode,
        revision: next,
        hash,
    })
}

pub(super) fn load_committed_definitions(
    connection: &Connection,
) -> Result<Vec<FlowDefinition>, StoreError> {
    let ids = {
        let mut statement = connection.prepare(
            "SELECT flow_id FROM flow_definitions WHERE flow_id NOT LIKE 'standalone/%' AND committed = 1 ORDER BY COALESCE(queue_order, 9223372036854775807), created_at, flow_id",
        )?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    ids.iter()
        .map(|id| load_flow_current(connection, id))
        .collect()
}

pub(super) fn source_state_with(
    connection: &Connection,
) -> Result<(FlowSourceMode, i64, String), StoreError> {
    let (mode, revision, hash): (String, i64, String) = connection.query_row(
        "SELECT flow_source_mode, flow_definition_revision, flow_definition_hash FROM settings WHERE id = 1", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok((
        mode.parse().map_err(StoreError::InvalidData)?,
        revision,
        hash,
    ))
}

pub(super) fn hash_committed_definitions(
    connection: &Connection,
    revision: i64,
) -> Result<String, StoreError> {
    Ok(
        FlowSourceDocument::from_definitions(revision, &load_committed_definitions(connection)?)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?
            .base
            .hash,
    )
}

fn reject_unready_definitions(connection: &Connection) -> Result<(), StoreError> {
    let (uncommitted, frozen, drafts): (i64, i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(CASE WHEN committed = 0 THEN 1 ELSE 0 END),0), COALESCE(SUM(CASE WHEN frozen = 1 THEN 1 ELSE 0 END),0), COALESCE(SUM(CASE WHEN draft_json IS NOT NULL THEN 1 ELSE 0 END),0) FROM flow_definitions WHERE flow_id NOT LIKE 'standalone/%'", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if uncommitted != 0 || frozen != 0 || drafts != 0 {
        return Err(StoreError::InvalidData(format!(
            "cannot enter sync source mode: {uncommitted} uncommitted, {frozen} frozen, {drafts} draft Flow definition(s); commit or finish/discard edits first"
        )));
    }
    Ok(())
}
