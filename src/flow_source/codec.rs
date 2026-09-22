use std::collections::BTreeSet;

use chrono_tz::Tz;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::flow::{
    Dependency, ExecutionMode, FlowDefinition, FlowTask, ScheduleSpec, parse_daily, parse_every,
    parse_first_at, parse_once, validate_definition,
};

use super::error::FlowSourceError;
use super::model::*;

const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_FLOWS: usize = 10_000;
const MAX_TASKS_TOTAL: usize = 100_000;
const MAX_TASKS_PER_FLOW: usize = 10_000;
const MAX_DEPENDENCIES_PER_TASK: usize = 10_000;

#[derive(Serialize)]
struct CanonicalPayload<'a> {
    schema_version: u32,
    flows: &'a [FlowSourceFlow],
}

pub fn parse_document(bytes: &[u8]) -> Result<FlowSourceDocument, FlowSourceError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(FlowSourceError::invalid(format!(
            "document exceeds the {} byte limit",
            MAX_DOCUMENT_BYTES
        )));
    }
    let document: FlowSourceDocument = serde_json::from_slice(bytes)?;
    document.validate()?;
    Ok(document)
}

pub fn canonical_hash(document: &FlowSourceDocument) -> Result<String, FlowSourceError> {
    document.validate()?;
    let normalized = document.normalized()?;
    let bytes = serde_json::to_vec(&CanonicalPayload {
        schema_version: normalized.schema_version,
        flows: &normalized.flows,
    })?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

pub fn format_document(document: &FlowSourceDocument) -> Result<Vec<u8>, FlowSourceError> {
    document.validate()?;
    let mut bytes = serde_json::to_vec_pretty(&document.normalized()?)?;
    bytes.push(b'\n');
    Ok(bytes)
}

impl FlowSourceDocument {
    pub fn from_definitions(
        revision: i64,
        definitions: &[FlowDefinition],
    ) -> Result<Self, FlowSourceError> {
        let flows = definitions
            .iter()
            .map(FlowSourceFlow::from_definition)
            .collect::<Result<Vec<_>, _>>()?;
        let mut document = Self {
            schema_version: FLOW_SOURCE_SCHEMA_VERSION,
            base: FlowSourceBase {
                revision,
                hash: format!("sha256:{}", "0".repeat(64)),
            },
            flows,
        };
        document.base.hash = canonical_hash(&document)?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), FlowSourceError> {
        if self.schema_version != FLOW_SOURCE_SCHEMA_VERSION {
            return Err(FlowSourceError::invalid(format!(
                "unsupported schema_version {}; expected {}",
                self.schema_version, FLOW_SOURCE_SCHEMA_VERSION
            )));
        }
        if self.base.revision < 0 {
            return Err(FlowSourceError::invalid(
                "base.revision must be zero or greater",
            ));
        }
        validate_hash(&self.base.hash)?;
        if self.flows.len() > MAX_FLOWS {
            return Err(FlowSourceError::invalid(format!(
                "document contains more than {MAX_FLOWS} flows"
            )));
        }

        let mut flow_ids = BTreeSet::new();
        let mut task_count = 0_usize;
        for flow in &self.flows {
            if flow.id.starts_with("standalone/") {
                return Err(FlowSourceError::invalid(format!(
                    "flow {:?} uses the reserved standalone/ prefix",
                    flow.id
                )));
            }
            if !flow_ids.insert(flow.id.as_str()) {
                return Err(FlowSourceError::invalid(format!(
                    "duplicate flow id {:?}",
                    flow.id
                )));
            }
            if flow.tasks.len() > MAX_TASKS_PER_FLOW {
                return Err(FlowSourceError::invalid(format!(
                    "flow {:?} contains more than {MAX_TASKS_PER_FLOW} tasks",
                    flow.id
                )));
            }
            task_count = task_count
                .checked_add(flow.tasks.len())
                .ok_or_else(|| FlowSourceError::invalid("document task count overflow"))?;
            if task_count > MAX_TASKS_TOTAL {
                return Err(FlowSourceError::invalid(format!(
                    "document contains more than {MAX_TASKS_TOTAL} tasks"
                )));
            }
            flow.validate()?;
        }
        Ok(())
    }

    fn normalized(&self) -> Result<Self, FlowSourceError> {
        Ok(Self {
            schema_version: self.schema_version,
            base: self.base.clone(),
            flows: self
                .flows
                .iter()
                .map(FlowSourceFlow::normalized)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl FlowSourceFlow {
    fn from_definition(definition: &FlowDefinition) -> Result<Self, FlowSourceError> {
        let schedule = definition.schedule.as_ref().ok_or_else(|| {
            FlowSourceError::invalid(format!("flow {:?} has no schedule", definition.flow_id))
        })?;
        Ok(Self {
            id: definition.flow_id.clone(),
            name: definition.name.clone(),
            owner: definition.owner.clone(),
            enabled: definition.enabled,
            schedule: FlowSourceSchedule::from_domain(schedule),
            tasks: definition
                .tasks
                .iter()
                .map(|task| FlowSourceTask {
                    id: task.task_id.clone(),
                    name: task.name.clone(),
                    cwd: Some(FlowSourceCwd::Path(task.cwd.clone())),
                    command: task.command.clone(),
                    retry: task.retry,
                    depend_mode: task.depend_mode,
                    depends_on: task
                        .dependencies
                        .iter()
                        .map(|edge| FlowSourceDependency {
                            task_id: edge.upstream_task_id.clone(),
                            status: edge.status,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    fn validate(&self) -> Result<(), FlowSourceError> {
        for task in &self.tasks {
            task.validate_cwd()?;
            if task.depends_on.len() > MAX_DEPENDENCIES_PER_TASK {
                return Err(FlowSourceError::invalid(format!(
                    "task {:?} in flow {:?} contains more than {MAX_DEPENDENCIES_PER_TASK} dependencies",
                    task.id, self.id
                )));
            }
        }
        validate_definition(&self.to_domain()?).map_err(FlowSourceError::invalid)
    }

    pub(super) fn to_domain(&self) -> Result<FlowDefinition, FlowSourceError> {
        let tasks = self
            .tasks
            .iter()
            .enumerate()
            .map(|(sequence, task)| FlowTask {
                task_id: task.id.clone(),
                name: task.name.clone(),
                cwd: ".".to_owned(),
                command: task.command.clone(),
                retry: task.retry,
                dependencies: task
                    .depends_on
                    .iter()
                    .map(|edge| Dependency {
                        upstream_task_id: edge.task_id.clone(),
                        status: edge.status,
                    })
                    .collect(),
                depend_mode: task.depend_mode,
                sequence: sequence as u64,
            })
            .collect();
        Ok(FlowDefinition {
            flow_id: self.id.clone(),
            internal_id: Uuid::nil(),
            name: self.name.clone(),
            owner: self.owner.clone(),
            mode: ExecutionMode::Scheduled,
            schedule: Some(self.schedule.to_domain()?),
            tasks,
            committed: true,
            frozen: false,
            enabled: self.enabled,
            graph_revision: 0,
            schedule_generation: 1,
            draft_revision: 0,
            has_draft: false,
            queue_order: None,
        })
    }

    fn normalized(&self) -> Result<Self, FlowSourceError> {
        let mut normalized = self.clone();
        normalized.schedule = FlowSourceSchedule::from_domain(&self.schedule.to_domain()?);
        for task in &mut normalized.tasks {
            task.depends_on.sort();
        }
        Ok(normalized)
    }
}

impl FlowSourceSchedule {
    fn from_domain(schedule: &ScheduleSpec) -> Self {
        match schedule {
            ScheduleSpec::Once { at } => Self::Once {
                at: at.to_rfc3339(),
            },
            ScheduleSpec::Daily { time, timezone } => Self::Daily {
                time: time.format("%H:%M").to_string(),
                timezone: timezone.clone(),
            },
            ScheduleSpec::Periodic { every, first_at } => Self::Periodic {
                every: every.to_string(),
                first_at: first_at.map(|value| value.to_rfc3339()),
            },
        }
    }

    pub(crate) fn to_domain(&self) -> Result<ScheduleSpec, FlowSourceError> {
        match self {
            Self::Once { at } => Ok(ScheduleSpec::Once {
                at: parse_once(at).map_err(FlowSourceError::invalid)?,
            }),
            Self::Daily { time, timezone } => {
                timezone.parse::<Tz>().map_err(|_| {
                    FlowSourceError::invalid(format!(
                        "unknown schedule timezone {timezone:?}; use an IANA timezone"
                    ))
                })?;
                Ok(ScheduleSpec::Daily {
                    time: parse_daily(time).map_err(FlowSourceError::invalid)?,
                    timezone: timezone.clone(),
                })
            }
            Self::Periodic { every, first_at } => Ok(ScheduleSpec::Periodic {
                every: parse_every(every).map_err(FlowSourceError::invalid)?,
                first_at: first_at
                    .as_deref()
                    .map(parse_first_at)
                    .transpose()
                    .map_err(FlowSourceError::invalid)?,
            }),
        }
    }
}

impl FlowSourceTask {
    fn validate_cwd(&self) -> Result<(), FlowSourceError> {
        match &self.cwd {
            None => Ok(()),
            Some(FlowSourceCwd::Path(path)) => validate_path_text(path, "cwd"),
            Some(FlowSourceCwd::Platform(paths)) => {
                let values = [
                    ("cwd.default", paths.default.as_deref()),
                    ("cwd.windows", paths.windows.as_deref()),
                    ("cwd.linux", paths.linux.as_deref()),
                    ("cwd.macos", paths.macos.as_deref()),
                ];
                if values.iter().all(|(_, value)| value.is_none()) {
                    return Err(FlowSourceError::invalid(format!(
                        "task {:?} cwd platform map must contain at least one path",
                        self.id
                    )));
                }
                for (field, value) in values {
                    if let Some(value) = value {
                        validate_path_text(value, field)?;
                    }
                }
                Ok(())
            }
        }
    }
}

fn validate_path_text(value: &str, field: &str) -> Result<(), FlowSourceError> {
    if value.trim().is_empty() {
        return Err(FlowSourceError::invalid(format!(
            "{field} must not be empty"
        )));
    }
    if value.contains('\0') {
        return Err(FlowSourceError::invalid(format!(
            "{field} must not contain a NUL character"
        )));
    }
    Ok(())
}

fn validate_hash(value: &str) -> Result<(), FlowSourceError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(FlowSourceError::invalid(
            "base.hash must use sha256:<64 lowercase hexadecimal digits>",
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(FlowSourceError::invalid(
            "base.hash must use sha256:<64 lowercase hexadecimal digits>",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveTime, TimeZone, Utc};

    use super::*;
    use crate::domain::flow::{
        DependencyMode, DependencyStatus, SchedulePeriod, SchedulePeriodUnit,
    };

    fn hash() -> String {
        format!("sha256:{}", "0".repeat(64))
    }

    fn task(id: &str) -> FlowSourceTask {
        FlowSourceTask {
            id: id.into(),
            name: id.into(),
            cwd: Some(FlowSourceCwd::Path(".".into())),
            command: format!("echo {id}"),
            retry: 0,
            depend_mode: DependencyMode::All,
            depends_on: vec![],
        }
    }

    fn document() -> FlowSourceDocument {
        let mut second = task("second");
        second.retry = 2;
        second.depends_on.push(FlowSourceDependency {
            task_id: "first".into(),
            status: DependencyStatus::Succeeded,
        });
        FlowSourceDocument {
            schema_version: FLOW_SOURCE_SCHEMA_VERSION,
            base: FlowSourceBase {
                revision: 0,
                hash: hash(),
            },
            flows: vec![FlowSourceFlow {
                id: "nightly".into(),
                name: "nightly".into(),
                owner: "alice".into(),
                enabled: true,
                schedule: FlowSourceSchedule::Daily {
                    time: "23:30".into(),
                    timezone: "Asia/Tokyo".into(),
                },
                tasks: vec![task("first"), second],
            }],
        }
    }

    #[test]
    fn pretty_document_round_trips_and_has_stable_hash() {
        let document = document();
        let bytes = format_document(&document).unwrap();
        let parsed = parse_document(&bytes).unwrap();
        assert_eq!(parsed, document);
        assert_eq!(
            canonical_hash(&parsed).unwrap(),
            canonical_hash(&document).unwrap()
        );
        assert!(bytes.ends_with(b"\n"));
    }

    #[test]
    fn whitespace_and_object_key_order_do_not_change_hash() {
        let first = format_document(&document()).unwrap();
        let reordered = format!(
            r#"{{"flows":[{{"tasks":[{{"depends_on":[],"depend_mode":"all","retry":0,"command":"echo first","cwd":".","name":"first","id":"first"}},{{"depends_on":[{{"status":"succeeded","task_id":"first"}}],"depend_mode":"all","retry":2,"command":"echo second","cwd":".","name":"second","id":"second"}}],"schedule":{{"timezone":"Asia/Tokyo","time":"23:30","type":"daily"}},"enabled":true,"owner":"alice","name":"nightly","id":"nightly"}}],"base":{{"hash":"{}","revision":0}},"schema_version":1}}"#,
            hash()
        );
        let first = parse_document(&first).unwrap();
        let second = parse_document(reordered.as_bytes()).unwrap();
        assert_eq!(
            canonical_hash(&first).unwrap(),
            canonical_hash(&second).unwrap()
        );
    }

    #[test]
    fn flow_and_task_order_change_hash_but_dependency_order_does_not() {
        let mut first = document();
        let mut third = task("third");
        third.depends_on = vec![
            FlowSourceDependency {
                task_id: "first".into(),
                status: DependencyStatus::Succeeded,
            },
            FlowSourceDependency {
                task_id: "second".into(),
                status: DependencyStatus::Failed,
            },
        ];
        first.flows[0].tasks.push(third);
        let mut dependency_order = first.clone();
        dependency_order.flows[0].tasks[2].depends_on.reverse();
        assert_eq!(
            canonical_hash(&first).unwrap(),
            canonical_hash(&dependency_order).unwrap()
        );

        let mut task_order = first.clone();
        task_order.flows[0].tasks.swap(0, 1);
        assert_ne!(
            canonical_hash(&first).unwrap(),
            canonical_hash(&task_order).unwrap()
        );

        let mut second_flow = first.flows[0].clone();
        second_flow.id = "other".into();
        first.flows.push(second_flow);
        let mut flow_order = first.clone();
        flow_order.flows.reverse();
        assert_ne!(
            canonical_hash(&first).unwrap(),
            canonical_hash(&flow_order).unwrap()
        );
    }

    #[test]
    fn parser_rejects_unknown_duplicate_and_missing_fields() {
        let valid = String::from_utf8(format_document(&document()).unwrap()).unwrap();
        let unknown = valid.replacen(
            "\"schema_version\": 1,",
            "\"schema_version\": 1,\n  \"unknown\": true,",
            1,
        );
        assert!(parse_document(unknown.as_bytes()).is_err());

        let duplicate = valid.replacen(
            "\"schema_version\": 1,",
            "\"schema_version\": 1,\n  \"schema_version\": 1,",
            1,
        );
        assert!(parse_document(duplicate.as_bytes()).is_err());

        let missing = valid.replacen("\"owner\": \"alice\",\n      ", "", 1);
        assert!(parse_document(missing.as_bytes()).is_err());

        let nested_unknown = valid.replacen(
            "\"command\": \"echo first\",",
            "\"command\": \"echo first\",\n          \"typo\": true,",
            1,
        );
        assert!(parse_document(nested_unknown.as_bytes()).is_err());
    }

    #[test]
    fn parser_rejects_versions_hashes_negative_revisions_and_large_input() {
        let mut value = serde_json::to_value(document()).unwrap();
        value["schema_version"] = serde_json::json!(0);
        assert!(parse_document(&serde_json::to_vec(&value).unwrap()).is_err());
        value["schema_version"] = serde_json::json!(2);
        assert!(parse_document(&serde_json::to_vec(&value).unwrap()).is_err());
        value["schema_version"] = serde_json::json!(1);
        value["base"]["revision"] = serde_json::json!(-1);
        assert!(parse_document(&serde_json::to_vec(&value).unwrap()).is_err());
        value["base"]["revision"] = serde_json::json!(0);
        for invalid in ["", "sha256:abc", &format!("SHA256:{}", "0".repeat(64))] {
            value["base"]["hash"] = serde_json::json!(invalid);
            assert!(parse_document(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let large = vec![b' '; MAX_DOCUMENT_BYTES + 1];
        let error = parse_document(&large).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn validation_rejects_invalid_graph_schedule_and_cwd_forms() {
        let mut duplicate_flow = document();
        duplicate_flow.flows.push(duplicate_flow.flows[0].clone());
        assert!(
            duplicate_flow
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate flow")
        );

        let mut reserved = document();
        reserved.flows[0].id = "standalone/id".into();
        assert!(reserved.validate().is_err());

        let mut missing = document();
        missing.flows[0].tasks[1].depends_on[0].task_id = "missing".into();
        assert!(missing.validate().is_err());

        let mut duplicate_task = document();
        duplicate_task.flows[0].tasks[1].id = "first".into();
        assert!(duplicate_task.validate().is_err());

        let mut contradictory = document();
        contradictory.flows[0].tasks[1]
            .depends_on
            .push(FlowSourceDependency {
                task_id: "first".into(),
                status: DependencyStatus::Failed,
            });
        assert!(contradictory.validate().is_err());

        let mut cycle = document();
        cycle.flows[0].tasks[0]
            .depends_on
            .push(FlowSourceDependency {
                task_id: "second".into(),
                status: DependencyStatus::Succeeded,
            });
        assert!(cycle.validate().is_err());

        let mut invalid_schedule = document();
        invalid_schedule.flows[0].schedule = FlowSourceSchedule::Periodic {
            every: "0m".into(),
            first_at: None,
        };
        assert!(invalid_schedule.validate().is_err());
        invalid_schedule.flows[0].schedule = FlowSourceSchedule::Daily {
            time: "25:00".into(),
            timezone: "Mars/Olympus".into(),
        };
        assert!(invalid_schedule.validate().is_err());

        let mut empty_map = document();
        empty_map.flows[0].tasks[0].cwd = Some(FlowSourceCwd::Platform(FlowSourceCwdMap {
            default: None,
            windows: None,
            linux: None,
            macos: None,
        }));
        assert!(empty_map.validate().is_err());
        empty_map.flows[0].tasks[0].cwd = Some(FlowSourceCwd::Path(" \t".into()));
        assert!(empty_map.validate().is_err());

        let mut empty_fields = document();
        empty_fields.flows[0].owner = " ".into();
        assert!(empty_fields.validate().is_err());
        empty_fields = document();
        empty_fields.flows[0].tasks[0].command = "".into();
        assert!(empty_fields.validate().is_err());
        empty_fields = document();
        empty_fields.flows[0].tasks[0].cwd = Some(FlowSourceCwd::Path("bad\0path".into()));
        assert!(empty_fields.validate().is_err());
    }

    #[test]
    fn validation_enforces_flow_and_task_count_limits() {
        let mut too_many_flows = document();
        too_many_flows.flows = (0..=MAX_FLOWS)
            .map(|index| {
                let mut flow = document().flows.remove(0);
                flow.id = format!("flow-{index}");
                flow
            })
            .collect();
        assert!(
            too_many_flows
                .validate()
                .unwrap_err()
                .to_string()
                .contains("flows")
        );

        let mut too_many_tasks = document();
        too_many_tasks.flows[0].tasks = (0..=MAX_TASKS_PER_FLOW)
            .map(|index| task(&format!("task-{index}")))
            .collect();
        assert!(
            too_many_tasks
                .validate()
                .unwrap_err()
                .to_string()
                .contains("tasks")
        );
    }

    #[test]
    fn definitions_export_all_schedule_and_dependency_fields() {
        let schedules = [
            ScheduleSpec::Once {
                at: Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap(),
            },
            ScheduleSpec::Daily {
                time: NaiveTime::from_hms_opt(23, 30, 0).unwrap(),
                timezone: "Asia/Tokyo".into(),
            },
            ScheduleSpec::Periodic {
                every: SchedulePeriod {
                    value: 15,
                    unit: SchedulePeriodUnit::Minutes,
                },
                first_at: Some(Utc.with_ymd_and_hms(2099, 1, 2, 0, 0, 0).unwrap()),
            },
        ];
        let definitions = schedules
            .into_iter()
            .enumerate()
            .map(|(index, schedule)| FlowDefinition {
                flow_id: format!("flow-{index}"),
                internal_id: Uuid::new_v4(),
                name: format!("flow-{index}"),
                owner: "alice".into(),
                mode: ExecutionMode::Scheduled,
                schedule: Some(schedule),
                tasks: vec![FlowTask {
                    task_id: "task".into(),
                    name: "task".into(),
                    cwd: "/work".into(),
                    command: "echo ok".into(),
                    retry: 3,
                    dependencies: vec![],
                    depend_mode: DependencyMode::Any,
                    sequence: 0,
                }],
                committed: true,
                frozen: false,
                enabled: index != 1,
                graph_revision: 8,
                schedule_generation: 9,
                draft_revision: 4,
                has_draft: false,
                queue_order: Some(index as i64),
            })
            .collect::<Vec<_>>();
        let document = FlowSourceDocument::from_definitions(7, &definitions).unwrap();
        assert_eq!(document.base.revision, 7);
        assert_eq!(document.base.hash, canonical_hash(&document).unwrap());
        assert_eq!(document.flows.len(), 3);
        assert_eq!(document.flows[0].tasks[0].retry, 3);
        assert_eq!(document.flows[0].tasks[0].depend_mode, DependencyMode::Any);
        assert!(!document.flows[1].enabled);
        assert!(matches!(
            document.flows[2].schedule,
            FlowSourceSchedule::Periodic { .. }
        ));
        assert!(parse_document(&format_document(&document).unwrap()).is_ok());
    }
}
