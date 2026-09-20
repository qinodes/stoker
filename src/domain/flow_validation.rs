//! Validation rules for persisted and declarative Flow definitions.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::flow::{
    ExecutionMode, FlowDefinition, FlowTask, MAX_FLOW_ID_LENGTH, MAX_FLOW_NAME_LENGTH,
    MAX_TASK_ID_LENGTH,
};

pub fn validate_definition(definition: &FlowDefinition) -> Result<(), String> {
    validate_flow_metadata(&definition.flow_id, &definition.name, &definition.owner)?;
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
        validate_task_metadata(&task.task_id, &task.name)?;
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
            validate_id(
                &edge.upstream_task_id,
                "dependency task-id",
                MAX_TASK_ID_LENGTH,
            )?;
            if !task_ids.contains(edge.upstream_task_id.as_str()) {
                return Err(format!(
                    "task {:?} depends on missing task {:?}",
                    task.task_id, edge.upstream_task_id
                ));
            }
            if !upstreams.insert(&edge.upstream_task_id) {
                return Err(format!(
                    "task {:?} has contradictory or duplicate dependency {:?}",
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
        return Err("scheduled flows require --once-at, --daily, or --every".to_owned());
    }
    Ok(())
}

pub fn validate_flow_metadata(flow_id: &str, name: &str, owner: &str) -> Result<(), String> {
    validate_id(flow_id, "flow-id", MAX_FLOW_ID_LENGTH)?;
    validate_display_name(name, "name", MAX_FLOW_NAME_LENGTH)?;
    if owner.trim().is_empty() {
        return Err("flow owner must not be empty".to_owned());
    }
    validate_no_control_characters(owner, "flow owner")
}

pub fn validate_task_metadata(task_id: &str, name: &str) -> Result<(), String> {
    validate_id(task_id, "task-id", MAX_TASK_ID_LENGTH)?;
    if name.trim().is_empty() {
        return Err(format!("task {task_id:?} name must not be empty"));
    }
    validate_no_control_characters(name, "task name")
}

fn validate_display_name(value: &str, field: &str, max: usize) -> Result<(), String> {
    let length = value.chars().count();
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if length > max {
        return Err(format!("{field} must be {max} characters or fewer"));
    }
    validate_no_control_characters(value, field)?;
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
    validate_no_control_characters(value, field)?;
    Ok(())
}

fn validate_no_control_characters(value: &str, field: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        return Err(format!("{field} must not contain a control character"));
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
