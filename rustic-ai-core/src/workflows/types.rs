use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    pub version: String,
    pub timeout_seconds: Option<u64>,
    pub entrypoints: BTreeMap<String, WorkflowEntrypoint>,
    pub steps: Vec<WorkflowStep>,
}

impl Default for WorkflowDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            version: "0.1.0".to_owned(),
            timeout_seconds: None,
            entrypoints: BTreeMap::new(),
            steps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkflowEntrypoint {
    pub step: String,
    pub triggers: WorkflowTriggerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkflowTriggerConfig {
    pub cron: Vec<String>,
    pub events: Vec<String>,
    pub webhooks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    pub kind: WorkflowStepKind,
    pub config: Value,
    pub outputs: BTreeMap<String, String>,
    pub next: Option<String>,
    pub on_success: Option<String>,
    pub on_failure: Option<String>,
    pub continue_on_error: bool,
}

impl Default for WorkflowStep {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: WorkflowStepKind::Tool,
            config: Value::Object(serde_json::Map::new()),
            outputs: BTreeMap::new(),
            next: None,
            on_success: None,
            on_failure: None,
            continue_on_error: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepKind {
    #[default]
    Tool,
    Skill,
    Agent,
    Workflow,
    Condition,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    #[default]
    Exists,
    Equals,
    NotEquals,
    Truthy,
    Falsy,
}
