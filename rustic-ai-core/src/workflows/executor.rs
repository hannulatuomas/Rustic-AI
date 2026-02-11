use super::expressions::{
    evaluate_expression_with_locals_and_options, evaluate_expression_with_options, is_truthy,
    EvaluationOptions,
};
use super::registry::WorkflowRegistry;
use super::types::{
    ConditionClause, ConditionGroup, ConditionOperator, LogicalOperator, NullHandlingMode,
    WorkflowExecutionConfig, WorkflowStep, WorkflowStepKind,
};
use crate::agents::AgentCoordinator;
use crate::config::schema::WorkflowCompatibilityPreset;
use crate::conversation::session_manager::SessionManager;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::skills::{SkillExecutionContext, SkillRegistry};
use crate::tools::ToolManager;
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt};
use regex::{Regex, RegexBuilder};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct WorkflowExecutorConfig {
    pub max_recursion_depth: Option<usize>,
    pub max_steps_per_run: Option<usize>,
    pub working_directory: PathBuf,
    pub default_timeout_seconds: u64,
    pub compatibility_preset: WorkflowCompatibilityPreset,
    pub switch_case_sensitive_default: Option<bool>,
    pub switch_pattern_priority: Option<String>,
    pub loop_continue_on_iteration_error_default: Option<bool>,
    pub wait_timeout_succeeds: Option<bool>,
    pub condition_missing_path_as_false: Option<bool>,
    pub default_continue_on_error: Option<bool>,
    pub continue_on_error_routing: Option<String>,
    pub execution_error_policy: Option<String>,
    pub timeout_error_policy: Option<String>,
    pub default_retry_count: Option<u32>,
    pub default_retry_backoff_ms: Option<u64>,
    pub default_retry_backoff_multiplier: Option<f64>,
    pub default_retry_backoff_max_ms: Option<u64>,
    pub condition_group_max_depth: usize,
    pub expression_max_length: usize,
    pub expression_max_depth: usize,
    pub loop_default_max_iterations: u64,
    pub loop_default_max_parallelism: u64,
    pub loop_hard_max_parallelism: u64,
    pub wait_default_poll_interval_ms: u64,
    pub wait_default_timeout_seconds: u64,
}

#[derive(Debug, Clone)]
struct EffectiveWorkflowConfig {
    max_recursion_depth: Option<usize>,
    max_steps_per_run: Option<usize>,
    condition_group_max_depth: usize,
    expression_max_length: usize,
    expression_max_depth: usize,
    loop_default_max_iterations: u64,
    loop_default_max_parallelism: u64,
    loop_hard_max_parallelism: u64,
    wait_default_poll_interval_ms: u64,
    wait_default_timeout_seconds: u64,
    null_handling: NullHandlingMode,
    switch_case_sensitive_default: bool,
    switch_pattern_priority: String,
    loop_continue_on_iteration_error_default: bool,
    wait_timeout_succeeds: bool,
    condition_missing_path_as_false: bool,
    default_continue_on_error: bool,
    continue_on_error_routing: String,
    execution_error_policy: String,
    timeout_error_policy: String,
    default_retry_count: u32,
    default_retry_backoff_ms: u64,
    default_retry_backoff_multiplier: f64,
    default_retry_backoff_max_ms: u64,
}

#[derive(Debug, Clone)]
struct TimeoutCheckContext<'a> {
    workflow_started_at: Instant,
    workflow_timeout_seconds: u64,
    step_started_at: Instant,
    step_timeout_seconds: Option<u64>,
    workflow_name: &'a str,
    step_id: &'a str,
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
    pub workflow_stack: Vec<String>,
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

    fn render_value_with_outputs(
        value: &Value,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
        expression_options: EvaluationOptions,
    ) -> Result<Value> {
        match value {
            Value::String(text) => {
                if Self::is_expression_candidate(text) {
                    let expression = Self::expression_from_template(text);
                    match evaluate_expression_with_options(expression, outputs, expression_options)
                    {
                        Ok(v) => Ok(v),
                        Err(err) => {
                            let mode = Self::expression_error_mode(step);
                            match mode {
                                "null" => Ok(Value::Null),
                                "literal" => Ok(Value::String(text.clone())),
                                "strict" => Err(Error::Tool(format!(
                                    "workflow step '{}' failed evaluating expression '{}': {err}",
                                    step.id, expression
                                ))),
                                other => Err(Error::Tool(format!(
                                    "workflow step '{}' has unsupported expression_error_mode '{}'",
                                    step.id, other
                                ))),
                            }
                        }
                    }
                } else {
                    Ok(Value::String(text.clone()))
                }
            }
            Value::Array(items) => {
                let mut rendered_items = Vec::with_capacity(items.len());
                for item in items {
                    rendered_items.push(Self::render_value_with_outputs(
                        item,
                        outputs,
                        step,
                        expression_options,
                    )?);
                }
                Ok(Value::Array(rendered_items))
            }
            Value::Object(map) => {
                let mut rendered = serde_json::Map::new();
                for (key, item) in map {
                    rendered.insert(
                        key.clone(),
                        Self::render_value_with_outputs(item, outputs, step, expression_options)?,
                    );
                }
                Ok(Value::Object(rendered))
            }
            _ => Ok(value.clone()),
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
            WorkflowStepKind::Wait => "wait",
            WorkflowStepKind::Loop => "loop",
            WorkflowStepKind::Merge => "merge",
            WorkflowStepKind::Switch => "switch",
        }
    }

    fn is_expression_candidate(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }

        if trimmed.starts_with("${") && trimmed.ends_with('}') {
            return true;
        }

        if trimmed.starts_with('$') {
            return true;
        }

        const MARKERS: [&str; 8] = ["==", "!=", ">=", "<=", " > ", " < ", "&&", "||"];
        if MARKERS.iter().any(|marker| trimmed.contains(marker))
            || trimmed.contains(" contains ")
            || trimmed.contains(" matches ")
        {
            return true;
        }

        if let Some(open_paren) = trimmed.find('(') {
            if trimmed.ends_with(')') {
                let name = trimmed[..open_paren].trim();
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                {
                    return true;
                }
            }
        }

        false
    }

    fn expression_from_template(text: &str) -> &str {
        let trimmed = text.trim();
        if trimmed.starts_with("${") && trimmed.ends_with('}') && trimmed.len() > 3 {
            &trimmed[2..trimmed.len() - 1]
        } else {
            trimmed
        }
    }

    fn expression_error_mode(step: &WorkflowStep) -> &str {
        step.config
            .get("expression_error_mode")
            .and_then(Value::as_str)
            .unwrap_or("strict")
    }

    fn effective_continue_on_error(
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> bool {
        step.config
            .get("continue_on_error")
            .and_then(Value::as_bool)
            .unwrap_or(step.continue_on_error || workflow_config.default_continue_on_error)
    }

    fn continue_on_error_routing<'a>(
        step: &'a WorkflowStep,
        workflow_config: &'a EffectiveWorkflowConfig,
    ) -> &'a str {
        step.config
            .get("continue_on_error_routing")
            .and_then(Value::as_str)
            .unwrap_or(&workflow_config.continue_on_error_routing)
    }

    fn execution_error_policy<'a>(
        step: &'a WorkflowStep,
        workflow_config: &'a EffectiveWorkflowConfig,
    ) -> &'a str {
        step.config
            .get("execution_error_policy")
            .and_then(Value::as_str)
            .unwrap_or(&workflow_config.execution_error_policy)
    }

    fn timeout_error_policy<'a>(
        step: &'a WorkflowStep,
        workflow_config: &'a EffectiveWorkflowConfig,
    ) -> &'a str {
        step.config
            .get("timeout_error_policy")
            .and_then(Value::as_str)
            .or_else(|| {
                step.config
                    .get("execution_error_policy")
                    .and_then(Value::as_str)
            })
            .unwrap_or(&workflow_config.timeout_error_policy)
    }

    fn retry_settings(
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> (u32, u64, f64, u64) {
        let retry_count = step
            .config
            .get("retry_count")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(workflow_config.default_retry_count)
            .min(20);
        let retry_backoff_ms = step
            .config
            .get("retry_backoff_ms")
            .and_then(Value::as_u64)
            .unwrap_or(workflow_config.default_retry_backoff_ms)
            .max(1);
        let retry_backoff_multiplier = step
            .config
            .get("retry_backoff_multiplier")
            .and_then(Value::as_f64)
            .unwrap_or(workflow_config.default_retry_backoff_multiplier)
            .clamp(1.0, 10.0);
        let retry_backoff_max_ms = step
            .config
            .get("retry_backoff_max_ms")
            .and_then(Value::as_u64)
            .unwrap_or(workflow_config.default_retry_backoff_max_ms)
            .max(retry_backoff_ms);
        (
            retry_count,
            retry_backoff_ms,
            retry_backoff_multiplier,
            retry_backoff_max_ms,
        )
    }

    fn next_backoff_ms(current: u64, multiplier: f64, max_ms: u64) -> u64 {
        let scaled = (current as f64 * multiplier).round();
        let scaled = if scaled.is_finite() && scaled > 0.0 {
            scaled as u64
        } else {
            current
        };
        scaled.clamp(1, max_ms)
    }

    fn ensure_within_timeout(
        started_at: Instant,
        timeout_seconds: u64,
        workflow_name: &str,
    ) -> Result<()> {
        if started_at.elapsed() >= Duration::from_secs(timeout_seconds) {
            return Err(Error::Timeout(format!(
                "workflow '{}' exceeded timeout of {} seconds",
                workflow_name, timeout_seconds
            )));
        }
        Ok(())
    }

    fn ensure_within_timeouts(
        event_tx: &mpsc::Sender<Event>,
        timeout_counter: &mut usize,
        ctx: &TimeoutCheckContext<'_>,
        timeout_error_policy: &str,
    ) -> Result<bool> {
        if let Err(err) = Self::ensure_within_timeout(
            ctx.workflow_started_at,
            ctx.workflow_timeout_seconds,
            ctx.workflow_name,
        ) {
            let _ = event_tx.try_send(Event::WorkflowTimeout {
                workflow: ctx.workflow_name.to_owned(),
                step_id: Some(ctx.step_id.to_owned()),
                timeout_seconds: ctx.workflow_timeout_seconds,
                scope: "workflow".to_owned(),
            });
            *timeout_counter += 1;
            return Err(err);
        }
        if let Some(step_timeout_seconds) = ctx.step_timeout_seconds {
            if ctx.step_started_at.elapsed() >= Duration::from_secs(step_timeout_seconds) {
                let _ = event_tx.try_send(Event::WorkflowTimeout {
                    workflow: ctx.workflow_name.to_owned(),
                    step_id: Some(ctx.step_id.to_owned()),
                    timeout_seconds: step_timeout_seconds,
                    scope: "step".to_owned(),
                });
                *timeout_counter += 1;
                if timeout_error_policy == "route_as_failure" {
                    return Ok(true);
                }
                return Err(Error::Timeout(format!(
                    "workflow '{}' step '{}' exceeded step timeout of {} seconds",
                    ctx.workflow_name, ctx.step_id, step_timeout_seconds
                )));
            }
        }
        Ok(false)
    }

    fn emit_retry_event(
        event_tx: &mpsc::Sender<Event>,
        workflow: &str,
        step_id: &str,
        attempt: u32,
        max_retries: u32,
        backoff_ms: u64,
        reason: &str,
    ) {
        let _ = event_tx.try_send(Event::WorkflowStepRetry {
            workflow: workflow.to_owned(),
            step_id: step_id.to_owned(),
            attempt,
            max_retries,
            backoff_ms,
            reason: reason.to_owned(),
        });
    }

    fn effective_config_for_workflow(
        &self,
        overrides: &WorkflowExecutionConfig,
    ) -> EffectiveWorkflowConfig {
        let preset_null_handling = match self.config.compatibility_preset {
            WorkflowCompatibilityPreset::N8n => NullHandlingMode::Lenient,
            WorkflowCompatibilityPreset::OpenCode
            | WorkflowCompatibilityPreset::ClaudeCode
            | WorkflowCompatibilityPreset::Rustic => NullHandlingMode::Strict,
        };
        let preset_switch_case_sensitive = !matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        );
        let preset_switch_pattern_priority = if matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        ) {
            "pattern_first"
        } else {
            "exact_first"
        };
        let preset_loop_continue_on_iteration_error = matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        );
        let preset_wait_timeout_succeeds = matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        );
        let preset_condition_missing_path_as_false = matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        );
        let preset_default_continue_on_error = matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        );
        let preset_continue_on_error_routing = "next_first";
        let preset_execution_error_policy = if matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        ) {
            "route_as_failure"
        } else {
            "abort"
        };
        let preset_timeout_error_policy = preset_execution_error_policy;
        let preset_default_retry_count = if matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        ) {
            2
        } else {
            0
        };
        let preset_default_retry_backoff_ms = if matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        ) {
            500
        } else {
            250
        };
        let preset_default_retry_backoff_multiplier = 2.0;
        let preset_default_retry_backoff_max_ms = if matches!(
            self.config.compatibility_preset,
            WorkflowCompatibilityPreset::N8n
        ) {
            30_000
        } else {
            10_000
        };

        EffectiveWorkflowConfig {
            max_recursion_depth: overrides
                .max_recursion_depth
                .or(self.config.max_recursion_depth),
            max_steps_per_run: overrides
                .max_steps_per_run
                .or(self.config.max_steps_per_run),
            condition_group_max_depth: overrides
                .condition_group_max_depth
                .unwrap_or(self.config.condition_group_max_depth),
            expression_max_length: overrides
                .expression_max_length
                .unwrap_or(self.config.expression_max_length),
            expression_max_depth: overrides
                .expression_max_depth
                .unwrap_or(self.config.expression_max_depth),
            loop_default_max_iterations: overrides
                .loop_default_max_iterations
                .unwrap_or(self.config.loop_default_max_iterations),
            loop_default_max_parallelism: overrides
                .loop_default_max_parallelism
                .unwrap_or(self.config.loop_default_max_parallelism),
            loop_hard_max_parallelism: overrides
                .loop_hard_max_parallelism
                .unwrap_or(self.config.loop_hard_max_parallelism),
            wait_default_poll_interval_ms: overrides
                .wait_default_poll_interval_ms
                .unwrap_or(self.config.wait_default_poll_interval_ms),
            wait_default_timeout_seconds: overrides
                .wait_default_timeout_seconds
                .unwrap_or(self.config.wait_default_timeout_seconds),
            null_handling: overrides.null_handling.unwrap_or(preset_null_handling),
            switch_case_sensitive_default: overrides
                .switch_case_sensitive_default
                .or(self.config.switch_case_sensitive_default)
                .unwrap_or(preset_switch_case_sensitive),
            switch_pattern_priority: overrides
                .switch_pattern_priority
                .clone()
                .or_else(|| self.config.switch_pattern_priority.clone())
                .unwrap_or_else(|| preset_switch_pattern_priority.to_owned()),
            loop_continue_on_iteration_error_default: overrides
                .loop_continue_on_iteration_error_default
                .or(self.config.loop_continue_on_iteration_error_default)
                .unwrap_or(preset_loop_continue_on_iteration_error),
            wait_timeout_succeeds: overrides
                .wait_timeout_succeeds
                .or(self.config.wait_timeout_succeeds)
                .unwrap_or(preset_wait_timeout_succeeds),
            condition_missing_path_as_false: overrides
                .condition_missing_path_as_false
                .or(self.config.condition_missing_path_as_false)
                .unwrap_or(preset_condition_missing_path_as_false),
            default_continue_on_error: overrides
                .default_continue_on_error
                .or(self.config.default_continue_on_error)
                .unwrap_or(preset_default_continue_on_error),
            continue_on_error_routing: overrides
                .continue_on_error_routing
                .clone()
                .or_else(|| self.config.continue_on_error_routing.clone())
                .unwrap_or_else(|| preset_continue_on_error_routing.to_owned()),
            execution_error_policy: overrides
                .execution_error_policy
                .clone()
                .or_else(|| self.config.execution_error_policy.clone())
                .unwrap_or_else(|| preset_execution_error_policy.to_owned()),
            timeout_error_policy: overrides
                .timeout_error_policy
                .clone()
                .or_else(|| self.config.timeout_error_policy.clone())
                .unwrap_or_else(|| preset_timeout_error_policy.to_owned()),
            default_retry_count: overrides
                .default_retry_count
                .or(self.config.default_retry_count)
                .unwrap_or(preset_default_retry_count),
            default_retry_backoff_ms: overrides
                .default_retry_backoff_ms
                .or(self.config.default_retry_backoff_ms)
                .unwrap_or(preset_default_retry_backoff_ms),
            default_retry_backoff_multiplier: overrides
                .default_retry_backoff_multiplier
                .or(self.config.default_retry_backoff_multiplier)
                .unwrap_or(preset_default_retry_backoff_multiplier),
            default_retry_backoff_max_ms: overrides
                .default_retry_backoff_max_ms
                .or(self.config.default_retry_backoff_max_ms)
                .unwrap_or(preset_default_retry_backoff_max_ms),
        }
    }

    fn null_handling_for_step(
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> NullHandlingMode {
        if let Some(mode) = step.config.get("null_handling").and_then(Value::as_str) {
            return match mode {
                "lenient" => NullHandlingMode::Lenient,
                _ => NullHandlingMode::Strict,
            };
        }
        workflow_config.null_handling
    }

    fn expression_options_for_step(
        &self,
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> EvaluationOptions {
        let max_length = step
            .config
            .get("expression_max_length")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(workflow_config.expression_max_length);
        let max_depth = step
            .config
            .get("expression_max_depth")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(workflow_config.expression_max_depth);
        EvaluationOptions {
            max_length,
            max_depth,
        }
    }

    fn evaluate_condition(
        &self,
        step: &WorkflowStep,
        outputs: &BTreeMap<String, Value>,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> Result<bool> {
        if let Some(group_value) = step.config.get("group") {
            let group: ConditionGroup =
                serde_json::from_value(group_value.clone()).map_err(|err| {
                    Error::Tool(format!(
                        "workflow condition step '{}' has invalid config.group: {err}",
                        step.id
                    ))
                })?;
            return self.evaluate_condition_group(&group, outputs, step, workflow_config, 1);
        }

        self.evaluate_condition_config(&step.config, outputs, step, workflow_config)
    }

    fn evaluate_condition_group(
        &self,
        group: &ConditionGroup,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
        depth: usize,
    ) -> Result<bool> {
        if depth > workflow_config.condition_group_max_depth {
            return Err(Error::Tool(format!(
                "workflow condition step '{}' exceeded condition group max depth {}",
                step.id, workflow_config.condition_group_max_depth
            )));
        }

        if group.conditions.is_empty() {
            return Ok(false);
        }

        match group.operator {
            LogicalOperator::And => {
                for condition in &group.conditions {
                    if !self.evaluate_condition_clause(
                        condition,
                        outputs,
                        step,
                        workflow_config,
                        depth + 1,
                    )? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            LogicalOperator::Or => {
                for condition in &group.conditions {
                    if self.evaluate_condition_clause(
                        condition,
                        outputs,
                        step,
                        workflow_config,
                        depth + 1,
                    )? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    fn evaluate_condition_clause(
        &self,
        clause: &ConditionClause,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
        depth: usize,
    ) -> Result<bool> {
        if let Some(group) = &clause.group {
            return self.evaluate_condition_group(group, outputs, step, workflow_config, depth);
        }

        if let Some(expression) = clause.expression.as_deref() {
            return self.evaluate_expression(expression, outputs, step, workflow_config);
        }

        let Some(path) = clause.path.as_deref() else {
            return Err(Error::Tool(format!(
                "workflow condition step '{}' has grouped clause without path/expression/group",
                step.id
            )));
        };

        let operator = clause.operator.unwrap_or(ConditionOperator::Exists);
        let mut config = serde_json::Map::new();
        config.insert("path".to_owned(), Value::String(path.to_owned()));
        config.insert(
            "operator".to_owned(),
            Value::String(
                match operator {
                    ConditionOperator::Exists => "exists",
                    ConditionOperator::Equals => "equals",
                    ConditionOperator::NotEquals => "not_equals",
                    ConditionOperator::GreaterThan => "greater_than",
                    ConditionOperator::GreaterThanOrEqual => "greater_than_or_equal",
                    ConditionOperator::LessThan => "less_than",
                    ConditionOperator::LessThanOrEqual => "less_than_or_equal",
                    ConditionOperator::Contains => "contains",
                    ConditionOperator::Matches => "matches",
                    ConditionOperator::Truthy => "truthy",
                    ConditionOperator::Falsy => "falsy",
                }
                .to_owned(),
            ),
        );
        if let Some(value) = &clause.value {
            config.insert("value".to_owned(), value.clone());
        }

        self.evaluate_condition_config(&Value::Object(config), outputs, step, workflow_config)
    }

    fn evaluate_condition_config(
        &self,
        config: &Value,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> Result<bool> {
        if let Some(expression) = config.get("expression").and_then(Value::as_str) {
            return self.evaluate_expression(expression, outputs, step, workflow_config);
        }

        let path = config.get("path").and_then(Value::as_str).ok_or_else(|| {
            Error::Tool(format!(
                "workflow condition step '{}' missing config.path",
                step.id
            ))
        })?;
        let op = config
            .get("operator")
            .and_then(Value::as_str)
            .unwrap_or("exists");
        let expected = config.get("value").cloned();
        let null_mode = Self::null_handling_for_step(step, workflow_config);

        let root = Self::outputs_root(outputs);
        let actual = Self::extract_path(&root, path);

        if actual.is_none()
            && !matches!(op, "exists" | "truthy" | "falsy")
            && null_mode == NullHandlingMode::Strict
        {
            if workflow_config.condition_missing_path_as_false {
                return Ok(false);
            }
            return Err(Error::Tool(format!(
                "workflow condition step '{}' missing path '{}' under strict null handling",
                step.id, path
            )));
        }

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
            ConditionOperator::GreaterThan => {
                Self::compare_values(&actual, &expected, step, ">", null_mode)?
            }
            ConditionOperator::GreaterThanOrEqual => {
                Self::compare_values(&actual, &expected, step, ">=", null_mode)?
            }
            ConditionOperator::LessThan => {
                Self::compare_values(&actual, &expected, step, "<", null_mode)?
            }
            ConditionOperator::LessThanOrEqual => {
                Self::compare_values(&actual, &expected, step, "<=", null_mode)?
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

    fn evaluate_expression(
        &self,
        expression: &str,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> Result<bool> {
        let options = self.expression_options_for_step(step, workflow_config);
        let value =
            evaluate_expression_with_options(expression, outputs, options).map_err(|err| {
                Error::Tool(format!(
                    "workflow condition step '{}' has invalid expression '{}': {err}",
                    step.id, expression
                ))
            })?;
        Ok(is_truthy(&value))
    }

    fn compare_values(
        actual: &Option<Value>,
        expected: &Option<Value>,
        step: &WorkflowStep,
        operator: &str,
        null_mode: NullHandlingMode,
    ) -> Result<bool> {
        let Some(actual) = actual else {
            if null_mode == NullHandlingMode::Lenient {
                return Ok(false);
            }
            return Err(Error::Tool(format!(
                "workflow condition step '{}' cannot apply operator '{}' because actual value is missing",
                step.id, operator
            )));
        };
        let Some(expected) = expected else {
            if null_mode == NullHandlingMode::Lenient {
                return Ok(false);
            }
            return Err(Error::Tool(format!(
                "workflow condition step '{}' cannot apply operator '{}' because expected value is missing",
                step.id, operator
            )));
        };

        if actual.is_null() || expected.is_null() {
            if null_mode == NullHandlingMode::Lenient {
                return Ok(false);
            }
            return Err(Error::Tool(format!(
                "workflow condition step '{}' cannot apply operator '{}' to null values under strict null handling",
                step.id, operator
            )));
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

        if null_mode == NullHandlingMode::Lenient {
            return Ok(false);
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

    fn switch_key_from_value(value: &Value) -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(value) => value.to_string(),
            Value::Null => "null".to_owned(),
            other => serde_json::to_string(other).unwrap_or_default(),
        }
    }

    fn build_regex(
        pattern: &str,
        flags: &str,
        step_id: &str,
        default_case_sensitive: bool,
    ) -> Result<Regex> {
        let mut builder = RegexBuilder::new(pattern);
        builder.case_insensitive(!default_case_sensitive);
        for flag in flags.chars() {
            match flag {
                'i' => {
                    builder.case_insensitive(true);
                }
                'm' => {
                    builder.multi_line(true);
                }
                's' => {
                    builder.dot_matches_new_line(true);
                }
                'U' => {
                    builder.swap_greed(true);
                }
                'u' => {
                    builder.unicode(true);
                }
                _ => {
                    return Err(Error::Tool(format!(
                        "workflow switch step '{}' has unsupported regex flag '{}'",
                        step_id, flag
                    )));
                }
            }
        }

        builder.build().map_err(|err| {
            Error::Tool(format!(
                "workflow switch step '{}' has invalid regex pattern '{}': {err}",
                step_id, pattern
            ))
        })
    }

    fn resolve_switch_target(
        step: &WorkflowStep,
        key: &str,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> Result<(Option<String>, String)> {
        let case_sensitive = step
            .config
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(workflow_config.switch_case_sensitive_default);
        let priority = step
            .config
            .get("pattern_priority")
            .and_then(Value::as_str)
            .unwrap_or(&workflow_config.switch_pattern_priority);

        let exact_match = || {
            step.config
                .get("cases")
                .and_then(Value::as_object)
                .and_then(|cases| {
                    if case_sensitive {
                        cases
                            .get(key)
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    } else {
                        cases.iter().find_map(|(case_key, target)| {
                            if case_key.eq_ignore_ascii_case(key) {
                                target.as_str().map(ToOwned::to_owned)
                            } else {
                                None
                            }
                        })
                    }
                })
        };

        let pattern_match = || -> Result<Option<String>> {
            if let Some(pattern_cases) = step.config.get("pattern_cases").and_then(Value::as_array)
            {
                for case in pattern_cases {
                    let pattern = case.get("pattern").and_then(Value::as_str).ok_or_else(|| {
                        Error::Tool(format!(
                            "workflow switch step '{}' pattern_cases entries must include 'pattern'",
                            step.id
                        ))
                    })?;
                    let target = case.get("target").and_then(Value::as_str).ok_or_else(|| {
                        Error::Tool(format!(
                            "workflow switch step '{}' pattern_cases entries must include 'target'",
                            step.id
                        ))
                    })?;
                    let flags = case.get("flags").and_then(Value::as_str).unwrap_or("");
                    let regex = Self::build_regex(pattern, flags, &step.id, case_sensitive)?;
                    if regex.is_match(key) {
                        return Ok(Some(target.to_owned()));
                    }
                }
            }
            Ok(None)
        };

        let matched = match priority {
            "pattern_first" => {
                if let Some(target) = pattern_match()? {
                    Some((target, "pattern".to_owned()))
                } else {
                    exact_match().map(|target| (target, "exact".to_owned()))
                }
            }
            _ => {
                if let Some(target) = exact_match() {
                    Some((target, "exact".to_owned()))
                } else {
                    pattern_match()?.map(|target| (target, "pattern".to_owned()))
                }
            }
        };

        if let Some((target, match_type)) = matched {
            return Ok((Some(target), match_type));
        }

        if let Some(target) = step
            .config
            .get("default")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        {
            return Ok((Some(target), "default".to_owned()));
        }

        if let Some(target) = step.next.clone() {
            return Ok((Some(target), "next".to_owned()));
        }

        Ok((None, "none".to_owned()))
    }

    async fn execute_loop_step(
        &self,
        step: &WorkflowStep,
        outputs: &BTreeMap<String, Value>,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> Result<Value> {
        let items_config = step.config.get("items").ok_or_else(|| {
            Error::Tool(format!(
                "workflow loop step '{}' missing config.items",
                step.id
            ))
        })?;

        let expression_options = self.expression_options_for_step(step, workflow_config);
        let items_value =
            Self::render_value_with_outputs(items_config, outputs, step, expression_options)?;
        let null_mode = Self::null_handling_for_step(step, workflow_config);
        let Value::Array(items) = items_value else {
            if null_mode == NullHandlingMode::Lenient && items_value.is_null() {
                return Ok(json!({
                    "count": 0,
                    "results": [],
                    "error_count": 0,
                    "errors": [],
                    "mode": "sequential",
                }));
            }
            return Err(Error::Tool(format!(
                "workflow loop step '{}' expected config.items to resolve to array",
                step.id
            )));
        };

        let max_iterations = step
            .config
            .get("max_iterations")
            .and_then(Value::as_u64)
            .unwrap_or(workflow_config.loop_default_max_iterations);
        if (items.len() as u64) > max_iterations {
            return Err(Error::Tool(format!(
                "workflow loop step '{}' has {} items exceeding max_iterations {}",
                step.id,
                items.len(),
                max_iterations
            )));
        }

        let item_variable = step
            .config
            .get("item_variable")
            .and_then(Value::as_str)
            .unwrap_or("item")
            .to_owned();
        let index_variable = step
            .config
            .get("index_variable")
            .and_then(Value::as_str)
            .unwrap_or("index")
            .to_owned();
        let result_expression = step
            .config
            .get("result_expression")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let parallel = step
            .config
            .get("parallel")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && result_expression.is_some();
        let max_parallelism =
            step.config
                .get("max_parallelism")
                .and_then(Value::as_u64)
                .unwrap_or(workflow_config.loop_default_max_parallelism)
                .clamp(1, workflow_config.loop_hard_max_parallelism) as usize;
        let continue_on_iteration_error = step
            .config
            .get("continue_on_iteration_error")
            .and_then(Value::as_bool)
            .unwrap_or(
                workflow_config.loop_continue_on_iteration_error_default || step.continue_on_error,
            );

        let mut results = Vec::with_capacity(items.len());
        let mut errors = Vec::new();

        if let Some(expression) = result_expression {
            if parallel {
                let outputs_owned = outputs.clone();
                let item_variable_owned = item_variable.clone();
                let index_variable_owned = index_variable.clone();
                let expression_owned = expression.clone();
                let step_id = step.id.clone();

                let mut evaluated = stream::iter(items.into_iter().enumerate().map(move |(index, item)| {
                    let outputs = outputs_owned.clone();
                    let item_variable = item_variable_owned.clone();
                    let index_variable = index_variable_owned.clone();
                    let expression = expression_owned.clone();
                    let step_id = step_id.clone();

                    async move {
                        let mut locals = BTreeMap::new();
                        locals.insert(item_variable, item);
                        locals.insert(index_variable, json!(index));
                        let value = evaluate_expression_with_locals_and_options(
                            &expression,
                            &outputs,
                            &locals,
                            expression_options,
                        )
                        .map_err(|err| {
                            format!(
                                "workflow loop step '{}' failed evaluating result expression for index {}: {err}",
                                step_id, index
                            )
                        });
                        (index, value)
                    }
                }))
                .buffer_unordered(max_parallelism)
                .collect::<Vec<_>>()
                .await;

                evaluated.sort_by_key(|(index, _)| *index);

                for (index, value) in evaluated {
                    match value {
                        Ok(value) => results.push(value),
                        Err(message) if continue_on_iteration_error => {
                            errors.push(json!({"index": index, "error": message}));
                            results.push(Value::Null);
                        }
                        Err(message) => {
                            return Err(Error::Tool(message));
                        }
                    }
                }
            } else {
                for (index, item) in items.into_iter().enumerate() {
                    let mut locals = BTreeMap::new();
                    locals.insert(item_variable.clone(), item.clone());
                    locals.insert(index_variable.clone(), json!(index));
                    let value = evaluate_expression_with_locals_and_options(
                        &expression,
                        outputs,
                        &locals,
                        expression_options,
                    )
                    .map_err(|err| {
                        Error::Tool(format!(
                            "workflow loop step '{}' failed evaluating result expression for index {}: {err}",
                            step.id, index
                        ))
                    });

                    match value {
                        Ok(value) => results.push(value),
                        Err(err) if continue_on_iteration_error => {
                            errors.push(json!({"index": index, "error": err.to_string()}));
                            results.push(Value::Null);
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
        } else {
            for item in items {
                results.push(item);
            }
        }

        Ok(json!({
            "count": results.len(),
            "results": results,
            "error_count": errors.len(),
            "errors": errors,
            "mode": if parallel { "parallel" } else { "sequential" },
        }))
    }

    fn execute_merge_step(
        &self,
        step: &WorkflowStep,
        outputs: &BTreeMap<String, Value>,
        workflow_config: &EffectiveWorkflowConfig,
    ) -> Result<Value> {
        let mode = step
            .config
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        let lenient_nulls =
            Self::null_handling_for_step(step, workflow_config) == NullHandlingMode::Lenient;
        let inputs = step
            .config
            .get("inputs")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                Error::Tool(format!(
                    "workflow merge step '{}' missing config.inputs object",
                    step.id
                ))
            })?;

        let mut resolved = BTreeMap::new();
        for (name, value) in inputs {
            resolved.insert(
                name.clone(),
                Self::render_value_with_outputs(
                    value,
                    outputs,
                    step,
                    self.expression_options_for_step(step, workflow_config),
                )?,
            );
        }

        match mode {
            "merge" => {
                let mut merged = serde_json::Map::new();
                for (name, value) in &resolved {
                    let Value::Object(map) = value else {
                        if lenient_nulls && value.is_null() {
                            continue;
                        }
                        if lenient_nulls {
                            continue;
                        }
                        return Err(Error::Tool(format!(
                            "workflow merge step '{}' input '{}' is not object for mode 'merge'",
                            step.id, name
                        )));
                    };
                    for (key, entry) in map {
                        merged.insert(key.clone(), entry.clone());
                    }
                }
                Ok(Value::Object(merged))
            }
            "append" => {
                let mut items = Vec::new();
                for (name, value) in &resolved {
                    let Value::Array(array) = value else {
                        if lenient_nulls && value.is_null() {
                            continue;
                        }
                        if lenient_nulls {
                            continue;
                        }
                        return Err(Error::Tool(format!(
                            "workflow merge step '{}' input '{}' is not array for mode 'append'",
                            step.id, name
                        )));
                    };
                    items.extend(array.iter().cloned());
                }
                Ok(Value::Array(items))
            }
            "combine" => Ok(serde_json::to_value(&resolved).unwrap_or(Value::Null)),
            "multiplex" => {
                let keys = resolved.keys().cloned().collect::<Vec<_>>();
                let max_len = resolved
                    .values()
                    .filter_map(Value::as_array)
                    .map(Vec::len)
                    .max()
                    .unwrap_or(0);

                let mut rows = Vec::with_capacity(max_len);
                for index in 0..max_len {
                    let mut row = serde_json::Map::new();
                    for key in &keys {
                        let value = resolved.get(key).cloned().unwrap_or(Value::Null);
                        let item = match value {
                            Value::Array(items) => items.get(index).cloned().unwrap_or(Value::Null),
                            other => other,
                        };
                        row.insert(key.clone(), item);
                    }
                    rows.push(Value::Object(row));
                }
                Ok(Value::Array(rows))
            }
            other => Err(Error::Tool(format!(
                "workflow merge step '{}' has unsupported mode '{}'",
                step.id, other
            ))),
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
            let workflow = self
                .workflows
                .get(&request.workflow_name)
                .ok_or_else(|| {
                    Error::NotFound(format!("workflow '{}' not found", request.workflow_name))
                })?
                .clone();
            let workflow_config = self.effective_config_for_workflow(&workflow.execution);
            let workflow_timeout_seconds = workflow
                .timeout_seconds
                .unwrap_or(self.config.default_timeout_seconds);
            let started_at = Instant::now();

            if let Some(max_depth) = workflow_config.max_recursion_depth {
                if request.recursion_depth > max_depth {
                    return Err(Error::Tool(format!(
                        "workflow recursion depth {} exceeded configured max_recursion_depth {}",
                        request.recursion_depth, max_depth
                    )));
                }
            }

            if request.workflow_stack.contains(&request.workflow_name) {
                let mut cycle_chain = request.workflow_stack.clone();
                cycle_chain.push(request.workflow_name.clone());
                return Err(Error::Tool(format!(
                    "workflow recursion cycle detected: {}",
                    cycle_chain.join(" -> ")
                )));
            }

            let mut workflow_stack = request.workflow_stack.clone();
            workflow_stack.push(request.workflow_name.clone());

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
            let mut retry_events = 0usize;
            let mut timeout_events = 0usize;

            let _ = event_tx.try_send(Event::WorkflowStarted {
                workflow: request.workflow_name.clone(),
                entrypoint: request.entrypoint.clone(),
                recursion_depth: request.recursion_depth,
            });

            loop {
                if let Err(err) = Self::ensure_within_timeout(
                    started_at,
                    workflow_timeout_seconds,
                    &request.workflow_name,
                ) {
                    let _ = event_tx.try_send(Event::WorkflowTimeout {
                        workflow: request.workflow_name.clone(),
                        step_id: None,
                        timeout_seconds: workflow_timeout_seconds,
                        scope: "workflow".to_owned(),
                    });
                    timeout_events += 1;
                    let _ = event_tx.try_send(Event::WorkflowCompleted {
                        workflow: request.workflow_name.clone(),
                        success: false,
                        steps_executed: step_count,
                        retries: retry_events,
                        timeouts: timeout_events,
                    });
                    return Err(err);
                }

                if let Some(max_steps) = workflow_config.max_steps_per_run {
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
                let step_started_at = Instant::now();
                let step_timeout_seconds = step
                    .config
                    .get("step_timeout_seconds")
                    .and_then(Value::as_u64);
                let timeout_ctx = TimeoutCheckContext {
                    workflow_started_at: started_at,
                    workflow_timeout_seconds,
                    step_started_at,
                    step_timeout_seconds,
                    workflow_name: &request.workflow_name,
                    step_id: &step.id,
                };

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
                        let args = Self::render_value_with_outputs(
                            &args_template,
                            &outputs,
                            &step,
                            self.expression_options_for_step(&step, &workflow_config),
                        )?;

                        let (retry_count, retry_backoff_ms, retry_multiplier, retry_backoff_max_ms) =
                            Self::retry_settings(&step, &workflow_config);
                        let execution_error_policy =
                            Self::execution_error_policy(&step, &workflow_config).to_owned();
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let mut attempt = 0u32;
                        let mut backoff_ms = retry_backoff_ms;
                        loop {
                            let step_cancellation = CancellationToken::new();
                            let call_future = tools.execute_tool_with_cancel(
                                request.session_id.clone(),
                                request.agent_name.clone(),
                                tool_name,
                                args.clone(),
                                event_tx.clone(),
                                Some(step_cancellation.clone()),
                            );
                            let call_result = if let Some(step_timeout_seconds) =
                                step_timeout_seconds
                            {
                                tokio::select! {
                                    result = call_future => result,
                                    _ = sleep(Duration::from_secs(step_timeout_seconds)) => {
                                        step_cancellation.cancel();
                                        let _ = event_tx.try_send(Event::WorkflowTimeout {
                                            workflow: request.workflow_name.clone(),
                                            step_id: Some(step.id.clone()),
                                            timeout_seconds: step_timeout_seconds,
                                            scope: "step".to_owned(),
                                        });
                                        timeout_events += 1;
                                        if timeout_error_policy == "route_as_failure" {
                                            break (
                                                false,
                                                json!({
                                                    "error": "step timeout",
                                                    "policy": "route_as_failure"
                                                }),
                                            );
                                        }
                                        return Err(Error::Timeout(format!(
                                            "workflow '{}' step '{}' exceeded step timeout of {} seconds",
                                            request.workflow_name, step.id, step_timeout_seconds
                                        )));
                                    }
                                }
                            } else {
                                call_future.await
                            };

                            match call_result {
                                Ok(Some(tool_result)) => {
                                    let parsed = Self::parse_step_result(&tool_result.output);
                                    if tool_result.success || attempt >= retry_count {
                                        break (tool_result.success, parsed);
                                    }
                                }
                                Ok(None) => {
                                    if attempt >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break (
                                                false,
                                                json!({
                                                    "error": "paused pending permission/user input",
                                                    "policy": "route_as_failure"
                                                }),
                                            );
                                        }
                                        return Err(Error::Tool(format!(
                                            "workflow '{}' step '{}' paused pending permission/user input; this flow is not resumable yet",
                                            request.workflow_name, step.id
                                        )));
                                    }
                                }
                                Err(err) => {
                                    if attempt >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break (
                                                false,
                                                json!({
                                                    "error": err.to_string(),
                                                    "policy": "route_as_failure"
                                                }),
                                            );
                                        }
                                        return Err(err);
                                    }
                                }
                            }

                            attempt += 1;
                            retry_events += 1;
                            Self::emit_retry_event(
                                &event_tx,
                                &request.workflow_name,
                                &step.id,
                                attempt,
                                retry_count,
                                backoff_ms,
                                "tool_step",
                            );
                            if Self::ensure_within_timeouts(
                                &event_tx,
                                &mut timeout_events,
                                &timeout_ctx,
                                &timeout_error_policy,
                            )? {
                                break (
                                    false,
                                    json!({
                                        "error": "step timeout",
                                        "policy": "route_as_failure"
                                    }),
                                );
                            }
                            sleep(Duration::from_millis(backoff_ms)).await;
                            backoff_ms = Self::next_backoff_ms(
                                backoff_ms,
                                retry_multiplier,
                                retry_backoff_max_ms,
                            );
                        }
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
                        let skill_input = Self::render_value_with_outputs(
                            &input_template,
                            &outputs,
                            &step,
                            self.expression_options_for_step(&step, &workflow_config),
                        )?;
                        let skill = self.skills.get(skill_name).ok_or_else(|| {
                            Error::NotFound(format!("skill '{}' not found", skill_name))
                        })?;
                        let (retry_count, retry_backoff_ms, retry_multiplier, retry_backoff_max_ms) =
                            Self::retry_settings(&step, &workflow_config);
                        let execution_error_policy =
                            Self::execution_error_policy(&step, &workflow_config).to_owned();
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let mut attempt = 0u32;
                        let mut backoff_ms = retry_backoff_ms;
                        loop {
                            let result = skill
                                .execute(
                                    skill_input.clone(),
                                    &SkillExecutionContext {
                                        working_directory: self.config.working_directory.clone(),
                                        environment: Default::default(),
                                    },
                                )
                                .await;

                            match result {
                                Ok(result) => {
                                    let parsed = Self::parse_step_result(&result.output);
                                    if result.success || attempt >= retry_count {
                                        break (result.success, parsed);
                                    }
                                }
                                Err(err) => {
                                    if attempt >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break (
                                                false,
                                                json!({
                                                    "error": err.to_string(),
                                                    "policy": "route_as_failure"
                                                }),
                                            );
                                        }
                                        return Err(err);
                                    }
                                }
                            }

                            attempt += 1;
                            retry_events += 1;
                            Self::emit_retry_event(
                                &event_tx,
                                &request.workflow_name,
                                &step.id,
                                attempt,
                                retry_count,
                                backoff_ms,
                                "skill_step",
                            );
                            if Self::ensure_within_timeouts(
                                &event_tx,
                                &mut timeout_events,
                                &timeout_ctx,
                                &timeout_error_policy,
                            )? {
                                break (
                                    false,
                                    json!({
                                        "error": "step timeout",
                                        "policy": "route_as_failure"
                                    }),
                                );
                            }
                            sleep(Duration::from_millis(backoff_ms)).await;
                            backoff_ms = Self::next_backoff_ms(
                                backoff_ms,
                                retry_multiplier,
                                retry_backoff_max_ms,
                            );
                        }
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
                        let nested_input = Self::render_value_with_outputs(
                            &nested_input_template,
                            &outputs,
                            &step,
                            self.expression_options_for_step(&step, &workflow_config),
                        )?;

                        let (retry_count, retry_backoff_ms, retry_multiplier, retry_backoff_max_ms) =
                            Self::retry_settings(&step, &workflow_config);
                        let execution_error_policy =
                            Self::execution_error_policy(&step, &workflow_config).to_owned();
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let mut attempt = 0u32;
                        let mut backoff_ms = retry_backoff_ms;
                        loop {
                            let nested = self
                                .run_internal(
                                    WorkflowRunRequest {
                                        workflow_name: nested_workflow.to_owned(),
                                        entrypoint: nested_entrypoint.to_owned(),
                                        session_id: request.session_id.clone(),
                                        agent_name: request.agent_name.clone(),
                                        input: nested_input.clone(),
                                        recursion_depth: request.recursion_depth + 1,
                                        workflow_stack: workflow_stack.clone(),
                                    },
                                    tools,
                                    event_tx.clone(),
                                )
                                .await;

                            match nested {
                                Ok(nested) => {
                                    let payload = json!({"outputs": nested.outputs, "steps_executed": nested.steps_executed});
                                    if nested.success || attempt >= retry_count {
                                        break (nested.success, payload);
                                    }
                                }
                                Err(err) => {
                                    if attempt >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break (
                                                false,
                                                json!({
                                                    "error": err.to_string(),
                                                    "policy": "route_as_failure"
                                                }),
                                            );
                                        }
                                        return Err(err);
                                    }
                                }
                            }

                            attempt += 1;
                            retry_events += 1;
                            Self::emit_retry_event(
                                &event_tx,
                                &request.workflow_name,
                                &step.id,
                                attempt,
                                retry_count,
                                backoff_ms,
                                "workflow_step",
                            );
                            if Self::ensure_within_timeouts(
                                &event_tx,
                                &mut timeout_events,
                                &timeout_ctx,
                                &timeout_error_policy,
                            )? {
                                break (
                                    false,
                                    json!({
                                        "error": "step timeout",
                                        "policy": "route_as_failure"
                                    }),
                                );
                            }
                            sleep(Duration::from_millis(backoff_ms)).await;
                            backoff_ms = Self::next_backoff_ms(
                                backoff_ms,
                                retry_multiplier,
                                retry_backoff_max_ms,
                            );
                        }
                    }
                    WorkflowStepKind::Condition => {
                        let (retry_count, retry_backoff_ms, retry_multiplier, retry_backoff_max_ms) =
                            Self::retry_settings(&step, &workflow_config);
                        let execution_error_policy =
                            Self::execution_error_policy(&step, &workflow_config).to_owned();
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let retry_on_false = step
                            .config
                            .get("retry_on_false")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let mut attempts = 0u32;
                        let mut backoff_ms = retry_backoff_ms;
                        let matched = loop {
                            match self.evaluate_condition(&step, &outputs, &workflow_config) {
                                Ok(matched) => {
                                    if !matched && retry_on_false && attempts < retry_count {
                                        attempts += 1;
                                        retry_events += 1;
                                        Self::emit_retry_event(
                                            &event_tx,
                                            &request.workflow_name,
                                            &step.id,
                                            attempts,
                                            retry_count,
                                            backoff_ms,
                                            "condition_false",
                                        );
                                        if Self::ensure_within_timeouts(
                                            &event_tx,
                                            &mut timeout_events,
                                            &timeout_ctx,
                                            &timeout_error_policy,
                                        )? {
                                            break false;
                                        }
                                        sleep(Duration::from_millis(backoff_ms)).await;
                                        backoff_ms = Self::next_backoff_ms(
                                            backoff_ms,
                                            retry_multiplier,
                                            retry_backoff_max_ms,
                                        );
                                        continue;
                                    }
                                    break matched;
                                }
                                Err(err) => {
                                    if attempts >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break false;
                                        }
                                        return Err(err);
                                    }
                                    attempts += 1;
                                    retry_events += 1;
                                    Self::emit_retry_event(
                                        &event_tx,
                                        &request.workflow_name,
                                        &step.id,
                                        attempts,
                                        retry_count,
                                        backoff_ms,
                                        "condition_error",
                                    );
                                    if Self::ensure_within_timeouts(
                                        &event_tx,
                                        &mut timeout_events,
                                        &timeout_ctx,
                                        &timeout_error_policy,
                                    )? {
                                        break false;
                                    }
                                    sleep(Duration::from_millis(backoff_ms)).await;
                                    backoff_ms = Self::next_backoff_ms(
                                        backoff_ms,
                                        retry_multiplier,
                                        retry_backoff_max_ms,
                                    );
                                }
                            }
                        };
                        outputs.insert(
                            format!("step.{}.condition_attempts", step.id),
                            Value::Number(serde_json::Number::from((attempts + 1) as u64)),
                        );
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
                            retries: retry_events,
                            timeouts: timeout_events,
                        });
                        outputs.insert("workflow.metrics.retries".to_owned(), json!(retry_events));
                        outputs.insert(
                            "workflow.metrics.timeouts".to_owned(),
                            json!(timeout_events),
                        );
                        return Ok(WorkflowExecutionResult {
                            success: matched,
                            outputs,
                            steps_executed: step_count,
                        });
                    }
                    WorkflowStepKind::Wait => {
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let mut waited_ms = 0u64;
                        let mut step_timed_out = false;
                        if let Some(seconds) =
                            step.config.get("duration_seconds").and_then(Value::as_u64)
                        {
                            let duration = Duration::from_secs(seconds);
                            if Self::ensure_within_timeouts(
                                &event_tx,
                                &mut timeout_events,
                                &timeout_ctx,
                                &timeout_error_policy,
                            )? {
                                step_timed_out = true;
                            }
                            if !step_timed_out {
                                sleep(duration).await;
                                waited_ms = waited_ms.saturating_add(duration.as_millis() as u64);
                            }
                        }

                        if step_timed_out {
                            (
                                false,
                                json!({
                                    "waited_ms": waited_ms,
                                    "timed_out": true,
                                    "timeout_scope": "step",
                                    "timeout_succeeds": false,
                                }),
                            )
                        } else if let Some(expression) =
                            step.config.get("until_expression").and_then(Value::as_str)
                        {
                            let poll_interval_ms = step
                                .config
                                .get("poll_interval_ms")
                                .and_then(Value::as_u64)
                                .unwrap_or(workflow_config.wait_default_poll_interval_ms);
                            let timeout_seconds = step
                                .config
                                .get("timeout_seconds")
                                .and_then(Value::as_u64)
                                .unwrap_or(workflow_config.wait_default_timeout_seconds);
                            let timeout_succeeds = step
                                .config
                                .get("timeout_succeeds")
                                .and_then(Value::as_bool)
                                .unwrap_or(workflow_config.wait_timeout_succeeds);
                            let started = Instant::now();
                            let mut condition_met = false;
                            let mut timed_out = false;

                            loop {
                                if self.evaluate_expression(
                                    expression,
                                    &outputs,
                                    &step,
                                    &workflow_config,
                                )? {
                                    condition_met = true;
                                    break;
                                }

                                if started.elapsed() >= Duration::from_secs(timeout_seconds) {
                                    timed_out = true;
                                    break;
                                }

                                if Self::ensure_within_timeouts(
                                    &event_tx,
                                    &mut timeout_events,
                                    &timeout_ctx,
                                    &timeout_error_policy,
                                )? {
                                    step_timed_out = true;
                                    break;
                                }
                                sleep(Duration::from_millis(poll_interval_ms)).await;
                                waited_ms = waited_ms.saturating_add(poll_interval_ms);
                            }

                            (
                                !step_timed_out
                                    && (condition_met || (timed_out && timeout_succeeds)),
                                json!({
                                    "waited_ms": waited_ms,
                                    "condition_met": condition_met,
                                    "timed_out": timed_out,
                                    "step_timed_out": step_timed_out,
                                    "timeout_succeeds": timeout_succeeds,
                                }),
                            )
                        } else {
                            (
                                true,
                                json!({
                                    "waited_ms": waited_ms,
                                }),
                            )
                        }
                    }
                    WorkflowStepKind::Loop => (
                        true,
                        self.execute_loop_step(&step, &outputs, &workflow_config)
                            .await?,
                    ),
                    WorkflowStepKind::Merge => (
                        true,
                        self.execute_merge_step(&step, &outputs, &workflow_config)?,
                    ),
                    WorkflowStepKind::Switch => {
                        let (retry_count, retry_backoff_ms, retry_multiplier, retry_backoff_max_ms) =
                            Self::retry_settings(&step, &workflow_config);
                        let execution_error_policy =
                            Self::execution_error_policy(&step, &workflow_config).to_owned();
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let retry_on_no_match = step
                            .config
                            .get("retry_on_no_match")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let value_template =
                            step.config.get("value").cloned().unwrap_or(Value::Null);
                        let mut attempts = 0u32;
                        let mut backoff_ms = retry_backoff_ms;
                        let (rendered_key, target, match_type) = loop {
                            let rendered = Self::render_value_with_outputs(
                                &value_template,
                                &outputs,
                                &step,
                                self.expression_options_for_step(&step, &workflow_config),
                            );
                            match rendered {
                                Ok(rendered) => {
                                    let rendered_key = Self::switch_key_from_value(&rendered);
                                    let resolved = Self::resolve_switch_target(
                                        &step,
                                        &rendered_key,
                                        &workflow_config,
                                    );
                                    match resolved {
                                        Ok((target, match_type)) => {
                                            if target.is_none()
                                                && retry_on_no_match
                                                && attempts < retry_count
                                            {
                                                attempts += 1;
                                                retry_events += 1;
                                                Self::emit_retry_event(
                                                    &event_tx,
                                                    &request.workflow_name,
                                                    &step.id,
                                                    attempts,
                                                    retry_count,
                                                    backoff_ms,
                                                    "switch_no_match",
                                                );
                                                if Self::ensure_within_timeouts(
                                                    &event_tx,
                                                    &mut timeout_events,
                                                    &timeout_ctx,
                                                    &timeout_error_policy,
                                                )? {
                                                    break (
                                                        rendered_key,
                                                        None,
                                                        "timeout".to_owned(),
                                                    );
                                                }
                                                sleep(Duration::from_millis(backoff_ms)).await;
                                                backoff_ms = Self::next_backoff_ms(
                                                    backoff_ms,
                                                    retry_multiplier,
                                                    retry_backoff_max_ms,
                                                );
                                                continue;
                                            }
                                            break (rendered_key, target, match_type);
                                        }
                                        Err(err) => {
                                            if attempts >= retry_count {
                                                if execution_error_policy == "route_as_failure" {
                                                    break (rendered_key, None, "error".to_owned());
                                                }
                                                return Err(err);
                                            }
                                            attempts += 1;
                                            retry_events += 1;
                                            Self::emit_retry_event(
                                                &event_tx,
                                                &request.workflow_name,
                                                &step.id,
                                                attempts,
                                                retry_count,
                                                backoff_ms,
                                                "switch_resolve_error",
                                            );
                                            if Self::ensure_within_timeouts(
                                                &event_tx,
                                                &mut timeout_events,
                                                &timeout_ctx,
                                                &timeout_error_policy,
                                            )? {
                                                break (rendered_key, None, "timeout".to_owned());
                                            }
                                            sleep(Duration::from_millis(backoff_ms)).await;
                                            backoff_ms = Self::next_backoff_ms(
                                                backoff_ms,
                                                retry_multiplier,
                                                retry_backoff_max_ms,
                                            );
                                        }
                                    }
                                }
                                Err(err) => {
                                    if attempts >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break (String::new(), None, "error".to_owned());
                                        }
                                        return Err(err);
                                    }
                                    attempts += 1;
                                    retry_events += 1;
                                    Self::emit_retry_event(
                                        &event_tx,
                                        &request.workflow_name,
                                        &step.id,
                                        attempts,
                                        retry_count,
                                        backoff_ms,
                                        "switch_render_error",
                                    );
                                    if Self::ensure_within_timeouts(
                                        &event_tx,
                                        &mut timeout_events,
                                        &timeout_ctx,
                                        &timeout_error_policy,
                                    )? {
                                        break (String::new(), None, "timeout".to_owned());
                                    }
                                    sleep(Duration::from_millis(backoff_ms)).await;
                                    backoff_ms = Self::next_backoff_ms(
                                        backoff_ms,
                                        retry_multiplier,
                                        retry_backoff_max_ms,
                                    );
                                }
                            }
                        };

                        let _ = event_tx.try_send(Event::WorkflowStepCompleted {
                            workflow: request.workflow_name.clone(),
                            step_id: step.id.clone(),
                            success: target.is_some(),
                            output_count: 1,
                        });

                        outputs.insert(
                            format!("step.{}.switch_key", step.id),
                            Value::String(rendered_key.clone()),
                        );
                        outputs.insert(
                            format!("step.{}.switch_match_type", step.id),
                            Value::String(match_type),
                        );
                        outputs.insert(
                            format!("step.{}.switch_attempts", step.id),
                            Value::Number(serde_json::Number::from((attempts + 1) as u64)),
                        );

                        if let Some(next) = target {
                            current = next;
                            continue;
                        }

                        let _ = event_tx.try_send(Event::WorkflowCompleted {
                            workflow: request.workflow_name.clone(),
                            success: true,
                            steps_executed: step_count,
                            retries: retry_events,
                            timeouts: timeout_events,
                        });
                        outputs.insert("workflow.metrics.retries".to_owned(), json!(retry_events));
                        outputs.insert(
                            "workflow.metrics.timeouts".to_owned(),
                            json!(timeout_events),
                        );
                        return Ok(WorkflowExecutionResult {
                            success: true,
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
                        let rendered_prompt = Self::render_value_with_outputs(
                            &prompt_template,
                            &outputs,
                            &step,
                            self.expression_options_for_step(&step, &workflow_config),
                        )?;
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
                        let (retry_count, retry_backoff_ms, retry_multiplier, retry_backoff_max_ms) =
                            Self::retry_settings(&step, &workflow_config);
                        let execution_error_policy =
                            Self::execution_error_policy(&step, &workflow_config).to_owned();
                        let timeout_error_policy =
                            Self::timeout_error_policy(&step, &workflow_config).to_owned();
                        let mut attempt = 0u32;
                        let mut backoff_ms = retry_backoff_ms;
                        loop {
                            let result = agent
                                .start_turn(session_uuid, prompt.clone(), event_tx.clone(), None)
                                .await;
                            match result {
                                Ok(()) => {
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
                                    break (true, Value::String(latest_assistant));
                                }
                                Err(err) => {
                                    if attempt >= retry_count {
                                        if execution_error_policy == "route_as_failure" {
                                            break (
                                                false,
                                                json!({
                                                    "error": err.to_string(),
                                                    "policy": "route_as_failure"
                                                }),
                                            );
                                        }
                                        return Err(err);
                                    }
                                }
                            }

                            attempt += 1;
                            retry_events += 1;
                            Self::emit_retry_event(
                                &event_tx,
                                &request.workflow_name,
                                &step.id,
                                attempt,
                                retry_count,
                                backoff_ms,
                                "agent_step",
                            );
                            if Self::ensure_within_timeouts(
                                &event_tx,
                                &mut timeout_events,
                                &timeout_ctx,
                                &timeout_error_policy,
                            )? {
                                break (
                                    false,
                                    json!({
                                        "error": "step timeout",
                                        "policy": "route_as_failure"
                                    }),
                                );
                            }
                            sleep(Duration::from_millis(backoff_ms)).await;
                            backoff_ms = Self::next_backoff_ms(
                                backoff_ms,
                                retry_multiplier,
                                retry_backoff_max_ms,
                            );
                        }
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

                let continue_on_error = Self::effective_continue_on_error(&step, &workflow_config);
                let failure_routing = Self::continue_on_error_routing(&step, &workflow_config);
                let next = if success {
                    step.on_success.or(step.next)
                } else if continue_on_error {
                    if failure_routing == "on_failure_first" {
                        step.on_failure.or(step.next)
                    } else {
                        step.next.or(step.on_failure)
                    }
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
                    retries: retry_events,
                    timeouts: timeout_events,
                });

                outputs.insert("workflow.metrics.retries".to_owned(), json!(retry_events));
                outputs.insert(
                    "workflow.metrics.timeouts".to_owned(),
                    json!(timeout_events),
                );

                return Ok(WorkflowExecutionResult {
                    success,
                    outputs,
                    steps_executed: step_count,
                });
            }
        })
    }
}
