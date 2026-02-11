use super::registry::WorkflowRegistry;
use super::types::{ConditionOperator, WorkflowStep, WorkflowStepKind};
use crate::agents::AgentCoordinator;
use crate::conversation::session_manager::SessionManager;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::skills::{SkillExecutionContext, SkillRegistry};
use crate::tools::ToolManager;
use futures::future::BoxFuture;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct WorkflowExecutorConfig {
    pub max_recursion_depth: Option<usize>,
    pub max_steps_per_run: Option<usize>,
    pub working_directory: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WorkflowExecutionResult {
    pub success: bool,
    pub outputs: BTreeMap<String, Value>,
    pub steps_executed: usize,
}

#[derive(Debug, Clone)]
pub struct WorkflowRunRequest {
    pub workflow_name: String,
    pub entrypoint: String,
    pub session_id: String,
    pub agent_name: Option<String>,
    pub input: Value,
    pub recursion_depth: usize,
}

#[derive(Clone)]
pub struct WorkflowExecutor {
    workflows: Arc<WorkflowRegistry>,
    skills: Arc<SkillRegistry>,
    agents: Arc<AgentCoordinator>,
    session_manager: Arc<SessionManager>,
    config: WorkflowExecutorConfig,
}

impl WorkflowExecutor {
    pub fn new(
        workflows: Arc<WorkflowRegistry>,
        skills: Arc<SkillRegistry>,
        agents: Arc<AgentCoordinator>,
        session_manager: Arc<SessionManager>,
        config: WorkflowExecutorConfig,
    ) -> Self {
        Self {
            workflows,
            skills,
            agents,
            session_manager,
            config,
        }
    }

    fn outputs_root(outputs: &BTreeMap<String, Value>) -> Value {
        json!(outputs)
    }

    fn extract_path(value: &Value, path: &str) -> Option<Value> {
        if path.trim().is_empty() {
            return None;
        }

        let mut current = value;
        let normalized = path.strip_prefix('$').unwrap_or(path);
        let normalized = normalized.strip_prefix('.').unwrap_or(normalized);
        if normalized.is_empty() {
            return Some(current.clone());
        }

        for part in normalized.split('.') {
            if part.is_empty() {
                continue;
            }
            current = current.get(part)?;
        }
        Some(current.clone())
    }

    fn render_value_with_outputs(value: &Value, outputs: &BTreeMap<String, Value>) -> Value {
        match value {
            Value::String(text) => {
                if text.starts_with("$") {
                    let root = Self::outputs_root(outputs);
                    Self::extract_path(&root, text).unwrap_or(Value::Null)
                } else {
                    Value::String(text.clone())
                }
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| Self::render_value_with_outputs(item, outputs))
                    .collect(),
            ),
            Value::Object(map) => {
                let mut rendered = serde_json::Map::new();
                for (key, item) in map {
                    rendered.insert(key.clone(), Self::render_value_with_outputs(item, outputs));
                }
                Value::Object(rendered)
            }
            _ => value.clone(),
        }
    }

    fn parse_step_result(raw: &str) -> Value {
        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
    }

    fn step_kind_name(kind: WorkflowStepKind) -> &'static str {
        match kind {
            WorkflowStepKind::Tool => "tool",
            WorkflowStepKind::Skill => "skill",
            WorkflowStepKind::Agent => "agent",
            WorkflowStepKind::Workflow => "workflow",
            WorkflowStepKind::Condition => "condition",
        }
    }

    fn evaluate_condition(step: &WorkflowStep, outputs: &BTreeMap<String, Value>) -> Result<bool> {
        if let Some(expression) = step.config.get("expression").and_then(Value::as_str) {
            return Self::evaluate_expression(expression, outputs, step);
        }

        let path = step
            .config
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Tool(format!(
                    "workflow condition step '{}' missing config.path",
                    step.id
                ))
            })?;
        let op = step
            .config
            .get("operator")
            .and_then(Value::as_str)
            .unwrap_or("exists");
        let expected = step.config.get("value").cloned();

        let root = Self::outputs_root(outputs);
        let actual = Self::extract_path(&root, path);

        let operator = match op {
            "exists" => ConditionOperator::Exists,
            "equals" => ConditionOperator::Equals,
            "not_equals" => ConditionOperator::NotEquals,
            "greater_than" => ConditionOperator::GreaterThan,
            "greater_than_or_equal" => ConditionOperator::GreaterThanOrEqual,
            "less_than" => ConditionOperator::LessThan,
            "less_than_or_equal" => ConditionOperator::LessThanOrEqual,
            "contains" => ConditionOperator::Contains,
            "matches" => ConditionOperator::Matches,
            "truthy" => ConditionOperator::Truthy,
            "falsy" => ConditionOperator::Falsy,
            other => {
                return Err(Error::Tool(format!(
                    "workflow condition step '{}' has unsupported operator '{}'",
                    step.id, other
                )));
            }
        };

        Ok(match operator {
            ConditionOperator::Exists => actual.is_some(),
            ConditionOperator::Equals => actual == expected,
            ConditionOperator::NotEquals => actual != expected,
            ConditionOperator::GreaterThan => Self::compare_values(&actual, &expected, step, ">")?,
            ConditionOperator::GreaterThanOrEqual => {
                Self::compare_values(&actual, &expected, step, ">=")?
            }
            ConditionOperator::LessThan => Self::compare_values(&actual, &expected, step, "<")?,
            ConditionOperator::LessThanOrEqual => {
                Self::compare_values(&actual, &expected, step, "<=")?
            }
            ConditionOperator::Contains => match (actual, expected) {
                (Some(a), Some(b)) => Self::contains_value(&a, &b),
                _ => false,
            },
            ConditionOperator::Matches => {
                let Some(Value::String(actual_text)) = actual else {
                    return Ok(false);
                };
                let Some(Value::String(pattern)) = expected else {
                    return Ok(false);
                };
                let regex = Regex::new(&pattern).map_err(|err| {
                    Error::Tool(format!(
                        "workflow condition step '{}' has invalid regex '{}': {err}",
                        step.id, pattern
                    ))
                })?;
                regex.is_match(&actual_text)
            }
            ConditionOperator::Truthy => actual
                .as_ref()
                .map(|value| match value {
                    Value::Bool(v) => *v,
                    Value::Number(v) => v.as_i64().unwrap_or_default() != 0,
                    Value::String(v) => !v.is_empty(),
                    Value::Array(v) => !v.is_empty(),
                    Value::Object(v) => !v.is_empty(),
                    Value::Null => false,
                })
                .unwrap_or(false),
            ConditionOperator::Falsy => actual
                .as_ref()
                .map(|value| match value {
                    Value::Bool(v) => !*v,
                    Value::Number(v) => v.as_i64().unwrap_or_default() == 0,
                    Value::String(v) => v.is_empty(),
                    Value::Array(v) => v.is_empty(),
                    Value::Object(v) => v.is_empty(),
                    Value::Null => true,
                })
                .unwrap_or(true),
        })
    }

    fn parse_literal_or_path(token: &str, outputs: &BTreeMap<String, Value>) -> Value {
        let trimmed = token.trim();
        if trimmed.starts_with('$') {
            let root = Self::outputs_root(outputs);
            return Self::extract_path(&root, trimmed).unwrap_or(Value::Null);
        }

        if ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
            && trimmed.len() >= 2
        {
            return Value::String(trimmed[1..trimmed.len() - 1].to_owned());
        }

        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            return v;
        }

        Value::String(trimmed.to_owned())
    }

    fn evaluate_expression(
        expression: &str,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
    ) -> Result<bool> {
        let operators = ["==", "!=", ">=", "<=", ">", "<", "contains", "matches"];
        for operator in operators {
            if let Some((left, right)) = expression.split_once(operator) {
                let left_value = Self::parse_literal_or_path(left, outputs);
                let right_value = Self::parse_literal_or_path(right, outputs);
                return match operator {
                    "==" => Ok(left_value == right_value),
                    "!=" => Ok(left_value != right_value),
                    ">" => {
                        Self::compare_values(&Some(left_value), &Some(right_value), step, operator)
                    }
                    ">=" => {
                        Self::compare_values(&Some(left_value), &Some(right_value), step, operator)
                    }
                    "<" => {
                        Self::compare_values(&Some(left_value), &Some(right_value), step, operator)
                    }
                    "<=" => {
                        Self::compare_values(&Some(left_value), &Some(right_value), step, operator)
                    }
                    "contains" => Ok(Self::contains_value(&left_value, &right_value)),
                    "matches" => {
                        let Value::String(left_text) = left_value else {
                            return Ok(false);
                        };
                        let Value::String(pattern) = right_value else {
                            return Ok(false);
                        };
                        let regex = Regex::new(&pattern).map_err(|err| {
                            Error::Tool(format!(
                                "workflow condition step '{}' has invalid expression regex '{}': {err}",
                                step.id, pattern
                            ))
                        })?;
                        Ok(regex.is_match(&left_text))
                    }
                    _ => Ok(false),
                };
            }
        }

        Ok(false)
    }

    fn compare_values(
        actual: &Option<Value>,
        expected: &Option<Value>,
        step: &WorkflowStep,
        operator: &str,
    ) -> Result<bool> {
        let Some(actual) = actual else {
            return Ok(false);
        };
        let Some(expected) = expected else {
            return Ok(false);
        };

        if let (Some(a), Some(b)) = (actual.as_f64(), expected.as_f64()) {
            return Ok(match operator {
                ">" => a > b,
                ">=" => a >= b,
                "<" => a < b,
                "<=" => a <= b,
                _ => false,
            });
        }

        if let (Some(a), Some(b)) = (actual.as_str(), expected.as_str()) {
            return Ok(match operator {
                ">" => a > b,
                ">=" => a >= b,
                "<" => a < b,
                "<=" => a <= b,
                _ => false,
            });
        }

        Err(Error::Tool(format!(
            "workflow condition step '{}' cannot apply operator '{}' to values '{}' and '{}'",
            step.id, operator, actual, expected
        )))
    }

    fn contains_value(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::String(a), Value::String(b)) => a.contains(b),
            (Value::Array(items), value) => items.iter().any(|item| item == value),
            (Value::Object(map), Value::String(key)) => map.contains_key(key),
            _ => false,
        }
    }

    fn map_named_outputs(
        step: &WorkflowStep,
        result: &Value,
        outputs: &mut BTreeMap<String, Value>,
    ) {
        for (name, path) in &step.outputs {
            if let Some(value) = Self::extract_path(result, path) {
                outputs.insert(name.clone(), value);
            }
        }
    }

    pub async fn run(
        &self,
        request: WorkflowRunRequest,
        tools: &ToolManager,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<WorkflowExecutionResult> {
        self.run_internal(request, tools, event_tx).await
    }

    fn run_internal<'a>(
        &'a self,
        request: WorkflowRunRequest,
        tools: &'a ToolManager,
        event_tx: mpsc::Sender<Event>,
    ) -> BoxFuture<'a, Result<WorkflowExecutionResult>> {
        Box::pin(async move {
            if let Some(max_depth) = self.config.max_recursion_depth {
                if request.recursion_depth > max_depth {
                    return Err(Error::Tool(format!(
                        "workflow recursion depth {} exceeded configured max_recursion_depth {}",
                        request.recursion_depth, max_depth
                    )));
                }
            }

            let workflow = self
                .workflows
                .get(&request.workflow_name)
                .ok_or_else(|| {
                    Error::NotFound(format!("workflow '{}' not found", request.workflow_name))
                })?
                .clone();

            let entry = workflow
                .entrypoints
                .get(&request.entrypoint)
                .ok_or_else(|| {
                    Error::NotFound(format!(
                        "workflow '{}' has no entrypoint '{}'",
                        request.workflow_name, request.entrypoint
                    ))
                })?;

            let mut by_id = HashMap::<String, WorkflowStep>::new();
            for step in &workflow.steps {
                by_id.insert(step.id.clone(), step.clone());
            }

            let mut outputs = BTreeMap::<String, Value>::new();
            outputs.insert("input".to_owned(), request.input);
            let mut current = entry.step.clone();
            let mut step_count = 0usize;

            let _ = event_tx.try_send(Event::WorkflowStarted {
                workflow: request.workflow_name.clone(),
                entrypoint: request.entrypoint.clone(),
                recursion_depth: request.recursion_depth,
            });

            loop {
                if let Some(max_steps) = self.config.max_steps_per_run {
                    if step_count >= max_steps {
                        return Err(Error::Tool(format!(
                            "workflow '{}' exceeded max_steps_per_run ({})",
                            request.workflow_name, max_steps
                        )));
                    }
                }
                step_count += 1;

                let step = by_id.get(&current).cloned().ok_or_else(|| {
                    Error::Tool(format!(
                        "workflow '{}' references missing step '{}'",
                        request.workflow_name, current
                    ))
                })?;

                let _ = event_tx.try_send(Event::WorkflowStepStarted {
                    workflow: request.workflow_name.clone(),
                    step_id: step.id.clone(),
                    step_name: step.name.clone(),
                    kind: Self::step_kind_name(step.kind).to_owned(),
                });

                let step_result = match step.kind {
                    WorkflowStepKind::Tool => {
                        let tool_name = step
                            .config
                            .get("tool")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                Error::Tool(format!(
                                    "workflow '{}' step '{}' missing config.tool",
                                    request.workflow_name, step.id
                                ))
                            })?;
                        let args_template = step
                            .config
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
                        let args = Self::render_value_with_outputs(&args_template, &outputs);

                        let tool_result = tools
                            .execute_tool(
                                request.session_id.clone(),
                                request.agent_name.clone(),
                                tool_name,
                                args,
                                event_tx.clone(),
                            )
                            .await?;
                        let Some(tool_result) = tool_result else {
                            return Err(Error::Tool(format!(
                                "workflow '{}' step '{}' paused pending permission/user input; this flow is not resumable yet",
                                request.workflow_name, step.id
                            )));
                        };

                        let parsed = Self::parse_step_result(&tool_result.output);
                        (tool_result.success, parsed)
                    }
                    WorkflowStepKind::Skill => {
                        let skill_name = step
                            .config
                            .get("skill")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                Error::Tool(format!(
                                    "workflow '{}' step '{}' missing config.skill",
                                    request.workflow_name, step.id
                                ))
                            })?;
                        let input_template = step
                            .config
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
                        let skill_input =
                            Self::render_value_with_outputs(&input_template, &outputs);
                        let skill = self.skills.get(skill_name).ok_or_else(|| {
                            Error::NotFound(format!("skill '{}' not found", skill_name))
                        })?;
                        let result = skill
                            .execute(
                                skill_input,
                                &SkillExecutionContext {
                                    working_directory: self.config.working_directory.clone(),
                                    environment: Default::default(),
                                },
                            )
                            .await?;
                        (result.success, Self::parse_step_result(&result.output))
                    }
                    WorkflowStepKind::Workflow => {
                        let nested_workflow = step
                            .config
                            .get("workflow")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                            Error::Tool(format!(
                                "workflow '{}' step '{}' missing config.workflow",
                                request.workflow_name, step.id
                            ))
                        })?;
                        let nested_entrypoint = step
                            .config
                            .get("entrypoint")
                            .and_then(Value::as_str)
                            .unwrap_or("start");
                        let nested_input_template = step
                            .config
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
                        let nested_input =
                            Self::render_value_with_outputs(&nested_input_template, &outputs);

                        let nested = self
                            .run_internal(
                                WorkflowRunRequest {
                                    workflow_name: nested_workflow.to_owned(),
                                    entrypoint: nested_entrypoint.to_owned(),
                                    session_id: request.session_id.clone(),
                                    agent_name: request.agent_name.clone(),
                                    input: nested_input,
                                    recursion_depth: request.recursion_depth + 1,
                                },
                                tools,
                                event_tx.clone(),
                            )
                            .await?;
                        (
                            nested.success,
                            json!({"outputs": nested.outputs, "steps_executed": nested.steps_executed}),
                        )
                    }
                    WorkflowStepKind::Condition => {
                        let matched = Self::evaluate_condition(&step, &outputs)?;
                        let _ = event_tx.try_send(Event::WorkflowStepCompleted {
                            workflow: request.workflow_name.clone(),
                            step_id: step.id.clone(),
                            success: matched,
                            output_count: 0,
                        });
                        let target = if matched {
                            step.on_success.clone().or(step.next.clone())
                        } else {
                            step.on_failure.clone().or(step.next.clone())
                        };
                        if let Some(next) = target {
                            current = next;
                            continue;
                        }
                        let _ = event_tx.try_send(Event::WorkflowCompleted {
                            workflow: request.workflow_name.clone(),
                            success: matched,
                            steps_executed: step_count,
                        });
                        return Ok(WorkflowExecutionResult {
                            success: matched,
                            outputs,
                            steps_executed: step_count,
                        });
                    }
                    WorkflowStepKind::Agent => {
                        let target_agent = step
                            .config
                            .get("agent")
                            .and_then(Value::as_str)
                            .or(request.agent_name.as_deref())
                            .ok_or_else(|| {
                                Error::Tool(format!(
                                    "workflow '{}' step '{}' missing config.agent and no default agent provided",
                                    request.workflow_name, step.id
                                ))
                            })?;

                        let prompt_template = step
                            .config
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| Value::String(String::new()));
                        let rendered_prompt =
                            Self::render_value_with_outputs(&prompt_template, &outputs);
                        let prompt = match rendered_prompt {
                            Value::String(text) => text,
                            other => serde_json::to_string(&other).unwrap_or_default(),
                        };

                        let session_uuid =
                            uuid::Uuid::parse_str(&request.session_id).map_err(|err| {
                                Error::Tool(format!(
                                    "workflow '{}' step '{}' requires UUID session id for agent calls: {err}",
                                    request.workflow_name, step.id
                                ))
                            })?;

                        let agent = self.agents.get_agent(Some(target_agent))?;
                        agent
                            .start_turn(session_uuid, prompt, event_tx.clone())
                            .await?;

                        let messages = self
                            .session_manager
                            .get_session_messages(session_uuid)
                            .await?;
                        let latest_assistant = messages
                            .iter()
                            .rev()
                            .find(|msg| msg.role == "assistant")
                            .map(|msg| msg.content.clone())
                            .unwrap_or_default();
                        (true, Value::String(latest_assistant))
                    }
                };

                let (success, payload) = step_result;
                Self::map_named_outputs(&step, &payload, &mut outputs);
                outputs.insert(format!("step.{}.result", step.id), payload);

                let _ = event_tx.try_send(Event::WorkflowStepCompleted {
                    workflow: request.workflow_name.clone(),
                    step_id: step.id.clone(),
                    success,
                    output_count: step.outputs.len(),
                });

                let next = if success {
                    step.on_success.or(step.next)
                } else if step.continue_on_error {
                    step.next.or(step.on_failure)
                } else {
                    step.on_failure
                };

                if let Some(next) = next {
                    current = next;
                    continue;
                }

                let _ = event_tx.try_send(Event::WorkflowCompleted {
                    workflow: request.workflow_name.clone(),
                    success,
                    steps_executed: step_count,
                });

                return Ok(WorkflowExecutionResult {
                    success,
                    outputs,
                    steps_executed: step_count,
                });
            }
        })
    }
}
