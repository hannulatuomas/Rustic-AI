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
    pub execution: WorkflowExecutionConfig,
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
            execution: WorkflowExecutionConfig::default(),
            entrypoints: BTreeMap::new(),
            steps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkflowExecutionConfig {
    pub max_recursion_depth: Option<usize>,
    pub max_steps_per_run: Option<usize>,
    pub condition_group_max_depth: Option<usize>,
    pub expression_max_length: Option<usize>,
    pub expression_max_depth: Option<usize>,
    pub loop_default_max_iterations: Option<u64>,
    pub loop_default_max_parallelism: Option<u64>,
    pub loop_hard_max_parallelism: Option<u64>,
    pub wait_default_poll_interval_ms: Option<u64>,
    pub wait_default_timeout_seconds: Option<u64>,
    pub null_handling: Option<NullHandlingMode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NullHandlingMode {
    #[default]
    Strict,
    Lenient,
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
    Wait,
    Loop,
    Merge,
    Switch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogicalOperator {
    #[default]
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConditionClause {
    pub path: Option<String>,
    pub operator: Option<ConditionOperator>,
    pub value: Option<Value>,
    pub expression: Option<String>,
    pub group: Option<ConditionGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConditionGroup {
    pub operator: LogicalOperator,
    pub conditions: Vec<ConditionClause>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    #[default]
    Exists,
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Contains,
    Matches,
    Truthy,
    Falsy,
}
