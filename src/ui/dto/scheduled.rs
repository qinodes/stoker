use serde::{Deserialize, Serialize};

use crate::domain::flow::{
    Attempt, Dependency, DependencyMode, FlowDefinition, FlowRun, FlowTask, Occurrence,
    OccurrenceState, ScheduleSpec, TaskRun,
};
use crate::flow_source::FlowSourceDocument;
use crate::store::{FlowSourceState, FlowSyncResult, StandaloneDefinition};

use super::JobDto;

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct FlowSourceStateResponse {
    pub mode: String,
    pub revision: i64,
    pub hash: String,
}

impl From<FlowSourceState> for FlowSourceStateResponse {
    fn from(state: FlowSourceState) -> Self {
        Self {
            mode: state.mode.to_string(),
            revision: state.revision,
            hash: state.hash,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct FlowSourceExportResponse {
    pub document: FlowSourceDocument,
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct FlowSourceSnapshotResponse {
    pub document: FlowSourceDocument,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct FlowSyncDiffResponse {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct FlowSyncPreviewResponse {
    pub diff: FlowSyncDiffResponse,
    pub revision: i64,
    pub hash: String,
    pub changed: bool,
}

impl From<FlowSyncResult> for FlowSyncPreviewResponse {
    fn from(result: FlowSyncResult) -> Self {
        Self {
            diff: FlowSyncDiffResponse {
                added: result.diff.added,
                updated: result.diff.updated,
                removed: result.diff.removed,
                unchanged: result.diff.unchanged,
            },
            revision: result.revision,
            hash: result.hash,
            changed: result.changed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(in crate::ui) enum ScheduledScheduleDto {
    Once {
        at: String,
    },
    Daily {
        time: String,
        timezone: String,
    },
    Periodic {
        every: String,
        first_at: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledTaskDto {
    pub task_id: String,
    pub name: String,
    pub cwd: String,
    pub command: String,
    pub retry: u32,
    pub dependencies: Vec<Dependency>,
    pub depend_mode: DependencyMode,
    pub sequence: u64,
}

impl From<FlowTask> for ScheduledTaskDto {
    fn from(task: FlowTask) -> Self {
        Self {
            task_id: task.task_id,
            name: task.name,
            cwd: task.cwd,
            command: task.command,
            retry: task.retry,
            dependencies: task.dependencies,
            depend_mode: task.depend_mode,
            sequence: task.sequence,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledFlowDto {
    pub flow_id: String,
    pub name: String,
    pub owner: String,
    pub mode: String,
    pub schedule: Option<ScheduledScheduleDto>,
    pub tasks: Vec<ScheduledTaskDto>,
    pub committed: bool,
    pub frozen: bool,
    pub enabled: bool,
    pub graph_revision: i64,
    pub schedule_generation: i64,
    pub draft_revision: i64,
    pub has_draft: bool,
    pub queue_order: Option<i64>,
}

impl From<FlowDefinition> for ScheduledFlowDto {
    fn from(flow: FlowDefinition) -> Self {
        Self {
            flow_id: flow.flow_id,
            name: flow.name,
            owner: flow.owner,
            mode: flow.mode.to_string(),
            schedule: flow.schedule.map(ScheduledScheduleDto::from),
            tasks: flow.tasks.into_iter().map(ScheduledTaskDto::from).collect(),
            committed: flow.committed,
            frozen: flow.frozen,
            enabled: flow.enabled,
            graph_revision: flow.graph_revision,
            schedule_generation: flow.schedule_generation,
            draft_revision: flow.draft_revision,
            has_draft: flow.has_draft,
            queue_order: flow.queue_order,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledOccurrenceDto {
    pub occurrence_id: uuid::Uuid,
    pub flow_id: String,
    pub generation: i64,
    pub local_date: Option<String>,
    pub due_at: String,
    pub state: OccurrenceState,
    pub reason: Option<String>,
}

impl From<Occurrence> for ScheduledOccurrenceDto {
    fn from(occurrence: Occurrence) -> Self {
        Self {
            occurrence_id: occurrence.occurrence_id,
            flow_id: occurrence.flow_id,
            generation: occurrence.generation,
            local_date: occurrence.local_date.map(|value| value.to_string()),
            due_at: occurrence.due_at.to_rfc3339(),
            state: occurrence.state,
            reason: occurrence.reason,
        }
    }
}

impl From<ScheduleSpec> for ScheduledScheduleDto {
    fn from(schedule: ScheduleSpec) -> Self {
        match schedule {
            ScheduleSpec::Once { at } => Self::Once {
                at: at.to_rfc3339(),
            },
            ScheduleSpec::Daily { time, timezone } => Self::Daily {
                time: time.format("%H:%M").to_string(),
                timezone,
            },
            ScheduleSpec::Periodic { every, first_at } => Self::Periodic {
                every: every.to_string(),
                first_at: first_at.map(|value| value.to_rfc3339()),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct StandaloneDefinitionDto {
    pub mode: String,
    pub schedule: Option<ScheduledScheduleDto>,
    pub retry: u32,
    pub enabled: bool,
    pub generation: i64,
}

impl From<StandaloneDefinition> for StandaloneDefinitionDto {
    fn from(definition: StandaloneDefinition) -> Self {
        Self {
            mode: definition.mode.to_string(),
            schedule: definition.schedule.map(ScheduledScheduleDto::from),
            retry: definition.retry,
            enabled: definition.enabled,
            generation: definition.generation,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledJobDto {
    pub job: JobDto,
    pub definition: StandaloneDefinitionDto,
    pub flow_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledAttemptDto {
    pub attempt_id: uuid::Uuid,
    pub number: u32,
    pub state: crate::domain::flow::AttemptState,
    pub exit_code: Option<i32>,
    pub failure_kind: Option<String>,
    pub failure_detail: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl From<Attempt> for ScheduledAttemptDto {
    fn from(attempt: Attempt) -> Self {
        Self {
            attempt_id: attempt.attempt_id,
            number: attempt.number,
            state: attempt.state,
            exit_code: attempt.exit_code,
            failure_kind: attempt.failure_kind,
            failure_detail: attempt.failure_detail,
            started_at: attempt.started_at.map(|value| value.to_rfc3339()),
            finished_at: attempt.finished_at.map(|value| value.to_rfc3339()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledTaskRunDto {
    pub task_id: String,
    pub state: crate::domain::flow::TaskRunState,
    pub attempt_count: u32,
    pub next_attempt_at: Option<String>,
    pub cancel_requested: bool,
    pub attempts: Vec<ScheduledAttemptDto>,
}

impl ScheduledTaskRunDto {
    pub(in crate::ui) fn new(task: &TaskRun, attempts: Vec<Attempt>) -> Self {
        Self {
            task_id: task.task_id.clone(),
            state: task.state,
            attempt_count: task.attempt_count,
            next_attempt_at: task.next_attempt_at.map(|value| value.to_rfc3339()),
            cancel_requested: task.cancel_requested,
            attempts: attempts
                .into_iter()
                .map(ScheduledAttemptDto::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::ui) struct ScheduledRunDto {
    pub run_id: uuid::Uuid,
    pub flow_id: String,
    pub generation: i64,
    pub source: String,
    pub state: crate::domain::flow::FlowRunState,
    pub occurrence_id: Option<uuid::Uuid>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub tasks: Vec<ScheduledTaskRunDto>,
}

impl ScheduledRunDto {
    pub(in crate::ui) fn new(run: FlowRun, tasks: Vec<ScheduledTaskRunDto>) -> Self {
        Self {
            run_id: run.run_id,
            flow_id: run.flow_id,
            generation: run.generation,
            source: run.source,
            state: run.state,
            occurrence_id: run.occurrence_id,
            started_at: run.started_at.map(|value| value.to_rfc3339()),
            finished_at: run.finished_at.map(|value| value.to_rfc3339()),
            tasks,
        }
    }
}
