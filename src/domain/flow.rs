//! Domain values for multi-task flows and durable schedules.
//!
//! This module deliberately contains no storage, process, or CLI concerns.
//! The same values are used by the SQLite store, the CLI parser, and the
//! deterministic graph/retry tests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_FLOW_ID_LENGTH: usize = 128;
pub const MAX_TASK_ID_LENGTH: usize = 128;
pub const MAX_FLOW_NAME_LENGTH: usize = 128;
pub const DEFAULT_SCHEDULED_CONCURRENCY: u32 = 2;
pub const RETRY_DELAY: Duration = Duration::seconds(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    #[default]
    Serial,
    Scheduled,
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Serial => "serial",
            Self::Scheduled => "scheduled",
        })
    }
}

impl FromStr for ExecutionMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "serial" => Ok(Self::Serial),
            "scheduled" => Ok(Self::Scheduled),
            other => Err(format!(
                "unknown execution mode {other:?}; use serial or scheduled"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleSpec {
    Once { at: DateTime<Utc> },
    Daily { time: NaiveTime, timezone: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyOccurrence {
    pub local_date: NaiveDate,
    pub due_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyStatus {
    Succeeded,
    Failed,
}

impl fmt::Display for DependencyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        })
    }
}

impl FromStr for DependencyStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            other => Err(format!(
                "unknown dependency status {other:?}; use succeeded or failed"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DependencyMode {
    #[default]
    All,
    Any,
}

impl fmt::Display for DependencyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "all",
            Self::Any => "any",
        })
    }
}

impl FromStr for DependencyMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "any" => Ok(Self::Any),
            other => Err(format!("unknown dependency mode {other:?}; use all or any")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub upstream_task_id: String,
    pub status: DependencyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowTask {
    pub task_id: String,
    pub name: String,
    pub cwd: String,
    pub command: String,
    pub retry: u32,
    pub dependencies: Vec<Dependency>,
    pub depend_mode: DependencyMode,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub flow_id: String,
    pub internal_id: Uuid,
    pub name: String,
    pub owner: String,
    pub mode: ExecutionMode,
    pub schedule: Option<ScheduleSpec>,
    pub tasks: Vec<FlowTask>,
    pub committed: bool,
    pub frozen: bool,
    pub enabled: bool,
    pub graph_revision: i64,
    pub schedule_generation: i64,
    pub draft_revision: i64,
    pub queue_order: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskRunState {
    Waiting,
    Ready,
    Starting,
    Running,
    RetryWait,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
    Lost,
}

impl TaskRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped | Self::Lost
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowRunState {
    Starting,
    Running,
    Cancelling,
    Recovering,
    Succeeded,
    Failed,
    FailedToStart,
    Cancelled,
    Skipped,
    Lost,
}

impl FlowRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::FailedToStart
                | Self::Cancelled
                | Self::Skipped
                | Self::Lost
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttemptState {
    Starting,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowRun {
    pub run_id: Uuid,
    pub flow_id: String,
    pub generation: i64,
    pub source: String,
    pub state: FlowRunState,
    pub occurrence_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub tasks: Vec<TaskRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRun {
    pub run_id: Uuid,
    pub task_id: String,
    pub state: TaskRunState,
    pub attempt_count: u32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub attempt_id: Uuid,
    pub run_id: Uuid,
    pub task_id: String,
    pub number: u32,
    pub state: AttemptState,
    pub exit_code: Option<i32>,
    pub failure_kind: Option<String>,
    pub failure_detail: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OccurrenceState {
    Pending,
    Reserved,
    Uncertain,
    Started,
    Replaced,
    Expired,
    Superseded,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    pub occurrence_id: Uuid,
    pub flow_id: String,
    pub generation: i64,
    pub local_date: Option<NaiveDate>,
    pub due_at: DateTime<Utc>,
    pub state: OccurrenceState,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyEvaluation {
    Ready,
    Waiting,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
    Lost,
}

pub fn validate_definition(definition: &FlowDefinition) -> Result<(), String> {
    validate_id(&definition.flow_id, "flow-id", MAX_FLOW_ID_LENGTH)?;
    validate_display_name(&definition.name, "name", MAX_FLOW_NAME_LENGTH)?;
    if definition.owner.trim().is_empty() {
        return Err("flow owner must not be empty".to_owned());
    }
    if definition.tasks.is_empty() {
        return Err("flow must contain at least one task".to_owned());
    }
    let task_ids: BTreeSet<_> = definition
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect();
    if task_ids.len() != definition.tasks.len() {
        return Err("task-id must be unique within a flow".to_owned());
    }
    for task in &definition.tasks {
        validate_id(&task.task_id, "task-id", MAX_TASK_ID_LENGTH)?;
        if task.name.trim().is_empty() {
            return Err(format!("task {} name must not be empty", task.task_id));
        }
        if task.command.trim().is_empty() {
            return Err(format!("task {} command must not be empty", task.task_id));
        }
        if task
            .dependencies
            .iter()
            .any(|edge| edge.upstream_task_id == task.task_id)
        {
            return Err(format!("task {} cannot depend on itself", task.task_id));
        }
        let mut upstreams = BTreeSet::new();
        for edge in &task.dependencies {
            if !task_ids.contains(edge.upstream_task_id.as_str()) {
                return Err(format!(
                    "task {} depends on missing task {}",
                    task.task_id, edge.upstream_task_id
                ));
            }
            if !upstreams.insert(&edge.upstream_task_id) {
                return Err(format!(
                    "task {} has contradictory or duplicate dependency {}",
                    task.task_id, edge.upstream_task_id
                ));
            }
        }
    }
    if has_cycle(&definition.tasks) {
        return Err("flow dependencies must not contain a cycle".to_owned());
    }
    let hidden_standalone = definition.flow_id.starts_with("standalone/");
    if !hidden_standalone && definition.mode != ExecutionMode::Scheduled {
        return Err("user-defined flows must use scheduled mode".to_owned());
    }
    if definition.mode == ExecutionMode::Serial && definition.schedule.is_some() {
        return Err("serial standalone definitions cannot have a schedule".to_owned());
    }
    if definition.mode == ExecutionMode::Scheduled && definition.schedule.is_none() {
        return Err("scheduled flows require --at or --daily".to_owned());
    }
    Ok(())
}

fn validate_display_name(value: &str, field: &str, max: usize) -> Result<(), String> {
    let length = value.chars().count();
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if length > max {
        return Err(format!("{field} must be {max} characters or fewer"));
    }
    Ok(())
}

fn validate_id(value: &str, field: &str, max: usize) -> Result<(), String> {
    let length = value.chars().count();
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if length > max {
        return Err(format!("{field} must be {max} characters or fewer"));
    }
    if value.contains(':') {
        return Err(format!("{field} must not contain ':'"));
    }
    Ok(())
}

fn has_cycle(tasks: &[FlowTask]) -> bool {
    let by_id: BTreeMap<_, _> = tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect();
    let mut indegree = BTreeMap::new();
    let mut downstream: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for task in tasks {
        indegree.insert(task.task_id.as_str(), task.dependencies.len());
        for edge in &task.dependencies {
            downstream
                .entry(edge.upstream_task_id.as_str())
                .or_default()
                .push(task.task_id.as_str());
        }
    }
    let mut queue = VecDeque::from_iter(
        indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id)),
    );
    let mut visited = 0;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        for child in downstream.get(id).into_iter().flatten() {
            let degree = indegree.get_mut(child).expect("child exists");
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(child);
            }
        }
    }
    visited != by_id.len()
}

pub fn evaluate_dependencies(
    task: &FlowTask,
    outcomes: &BTreeMap<String, TaskOutcome>,
) -> DependencyEvaluation {
    if task.dependencies.is_empty() {
        return DependencyEvaluation::Ready;
    }
    let mut has_unknown = false;
    let mut has_true = false;
    let mut has_false = false;
    for edge in &task.dependencies {
        match outcomes.get(&edge.upstream_task_id) {
            None => has_unknown = true,
            Some(outcome)
                if matches!(
                    (outcome, edge.status),
                    (TaskOutcome::Succeeded, DependencyStatus::Succeeded)
                        | (TaskOutcome::Failed, DependencyStatus::Failed)
                ) =>
            {
                has_true = true
            }
            Some(_) => has_false = true,
        }
    }
    match task.depend_mode {
        DependencyMode::All if has_false => DependencyEvaluation::Skipped,
        DependencyMode::All if !has_unknown => DependencyEvaluation::Ready,
        DependencyMode::Any if has_true => DependencyEvaluation::Ready,
        DependencyMode::Any if !has_unknown && has_false => DependencyEvaluation::Skipped,
        _ => DependencyEvaluation::Waiting,
    }
}

pub fn aggregate_flow_state(tasks: &[TaskRun], cancel_requested: bool) -> Option<FlowRunState> {
    if tasks.iter().any(|task| !task.state.is_terminal()) {
        return None;
    }
    if tasks.iter().any(|task| task.state == TaskRunState::Lost) {
        return Some(FlowRunState::Lost);
    }
    if cancel_requested
        || tasks
            .iter()
            .any(|task| task.state == TaskRunState::Cancelled)
    {
        return Some(FlowRunState::Cancelled);
    }
    if tasks.iter().any(|task| task.state == TaskRunState::Failed) {
        return Some(FlowRunState::Failed);
    }
    if tasks
        .iter()
        .any(|task| task.state == TaskRunState::Succeeded)
    {
        return Some(FlowRunState::Succeeded);
    }
    Some(FlowRunState::Skipped)
}

pub use super::flow_parsing::{
    next_local_date_after, parse_daily, parse_once, resolve_schedule_timezone,
};
#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, dependencies: Vec<Dependency>, mode: DependencyMode) -> FlowTask {
        FlowTask {
            task_id: id.into(),
            name: id.into(),
            cwd: ".".into(),
            command: "true".into(),
            retry: 0,
            dependencies,
            depend_mode: mode,
            sequence: 0,
        }
    }

    #[test]
    fn parses_offsets_and_rejects_ambiguous_or_incomplete_times() {
        let a = parse_once("2026-09-15T08:00:00+09:00").unwrap();
        let b = parse_once("2026-09-14T23:00:00Z").unwrap();
        assert_eq!(a, b);
        assert!(parse_once("2026-09-15T08:00:00").is_err());
        assert!(parse_once("2026-09-15T08:00").is_err());
    }

    #[test]
    fn graph_validation_and_dependency_evaluation_cover_branches() {
        let a = task("a", vec![], DependencyMode::All);
        let b = task(
            "b",
            vec![Dependency {
                upstream_task_id: "a".into(),
                status: DependencyStatus::Succeeded,
            }],
            DependencyMode::All,
        );
        let mut definition = FlowDefinition {
            flow_id: "f".into(),
            internal_id: Uuid::new_v4(),
            name: "f".into(),
            owner: "u".into(),
            mode: ExecutionMode::Scheduled,
            schedule: Some(ScheduleSpec::Once { at: Utc::now() }),
            tasks: vec![a.clone(), b.clone()],
            committed: false,
            frozen: false,
            enabled: true,
            graph_revision: 0,
            schedule_generation: 0,
            draft_revision: 0,
            queue_order: None,
        };
        assert!(validate_definition(&definition).is_ok());
        let mut unsupported_serial_flow = definition.clone();
        unsupported_serial_flow.mode = ExecutionMode::Serial;
        unsupported_serial_flow.schedule = None;
        assert_eq!(
            validate_definition(&unsupported_serial_flow),
            Err("user-defined flows must use scheduled mode".into())
        );
        let outcomes = BTreeMap::new();
        assert_eq!(
            evaluate_dependencies(&b, &outcomes),
            DependencyEvaluation::Waiting
        );
        let outcomes = BTreeMap::from([(String::from("a"), TaskOutcome::Succeeded)]);
        assert_eq!(
            evaluate_dependencies(&b, &outcomes),
            DependencyEvaluation::Ready
        );
        definition.tasks[0].dependencies.push(Dependency {
            upstream_task_id: "b".into(),
            status: DependencyStatus::Succeeded,
        });
        assert!(validate_definition(&definition).is_err());
    }

    #[test]
    fn any_keeps_a_branch_when_another_dependency_is_cancelled() {
        let node = task(
            "c",
            vec![
                Dependency {
                    upstream_task_id: "a".into(),
                    status: DependencyStatus::Succeeded,
                },
                Dependency {
                    upstream_task_id: "b".into(),
                    status: DependencyStatus::Succeeded,
                },
            ],
            DependencyMode::Any,
        );
        let outcomes = BTreeMap::from([
            (String::from("a"), TaskOutcome::Cancelled),
            (String::from("b"), TaskOutcome::Succeeded),
        ]);
        assert_eq!(
            evaluate_dependencies(&node, &outcomes),
            DependencyEvaluation::Ready
        );
    }

    #[test]
    fn daily_dst_nonexistent_is_skipped_and_repeated_uses_earlier_instant() {
        let spring = ScheduleSpec::Daily {
            time: NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
            timezone: "America/New_York".into(),
        };
        assert!(
            spring
                .due_at(NaiveDate::from_ymd_opt(2026, 3, 8).unwrap())
                .unwrap()
                .is_none()
        );
        let fall = ScheduleSpec::Daily {
            time: NaiveTime::from_hms_opt(1, 30, 0).unwrap(),
            timezone: "America/New_York".into(),
        };
        let due = fall
            .due_at(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(due.to_rfc3339(), "2026-11-01T05:30:00+00:00");
    }
}
