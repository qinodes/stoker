use serde::{Deserialize, Serialize};

use crate::domain::flow::{DependencyMode, DependencyStatus};

pub const FLOW_SOURCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowSourceDocument {
    pub schema_version: u32,
    pub base: FlowSourceBase,
    pub flows: Vec<FlowSourceFlow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowSourceBase {
    pub revision: i64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowSourceFlow {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub enabled: bool,
    pub schedule: FlowSourceSchedule,
    pub tasks: Vec<FlowSourceTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum FlowSourceSchedule {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowSourceTask {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<FlowSourceCwd>,
    pub command: String,
    pub retry: u32,
    pub depend_mode: DependencyMode,
    pub depends_on: Vec<FlowSourceDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FlowSourceCwd {
    Path(String),
    Platform(FlowSourceCwdMap),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowSourceCwdMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linux: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macos: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowSourceDependency {
    pub task_id: String,
    pub status: DependencyStatus,
}
