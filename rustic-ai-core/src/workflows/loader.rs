use super::registry::WorkflowRegistry;
use super::types::{ConditionClause, ConditionGroup, WorkflowDefinition, WorkflowStepKind};
use crate::config::schema::WorkflowsConfig;
use crate::error::{Error, Result};
use jsonschema::JSONSchema;
use regex::RegexBuilder;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub struct WorkflowLoader;

impl WorkflowLoader {
    fn workflow_schema() -> Result<&'static JSONSchema> {
        static SCHEMA: OnceLock<std::result::Result<JSONSchema, String>> = OnceLock::new();
        let compiled = SCHEMA.get_or_init(|| {
            let raw = include_str!("../../../docs/workflow.definition.schema.json");
            let schema_value = serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|err| format!("invalid embedded workflow schema json: {err}"))?;
            JSONSchema::compile(&schema_value)
                .map_err(|err| format!("failed compiling embedded workflow schema: {err}"))
        });

        match compiled {
            Ok(schema) => Ok(schema),
            Err(err) => Err(Error::Config(err.clone())),
        }
    }

    fn parse_definition_value(path: &Path, raw: &str) -> Result<serde_json::Value> {
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();

        if matches!(ext, "yaml" | "yml") {
            let yaml_value = serde_yaml::from_str::<serde_yaml::Value>(raw).map_err(|err| {
                Error::Config(format!(
                    "failed parsing workflow yaml '{}': {err}",
                    path.display()
                ))
            })?;
            serde_json::to_value(yaml_value).map_err(|err| {
                Error::Config(format!(
                    "failed converting workflow yaml '{}' to json: {err}",
                    path.display()
                ))
            })
        } else {
            serde_json::from_str::<serde_json::Value>(raw).map_err(|err| {
                Error::Config(format!(
                    "failed parsing workflow json '{}': {err}",
                    path.display()
                ))
            })
        }
    }

    fn validate_against_schema(path: &Path, value: &serde_json::Value) -> Result<()> {
        let schema = Self::workflow_schema()?;
        if let Err(errors) = schema.validate(value) {
            let details = errors
                .map(|err| format!("{}: {}", err.instance_path, err))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(Error::Validation(format!(
                "workflow '{}' failed schema validation: {}",
                path.display(),
                details
            )));
        }
        Ok(())
    }

    fn condition_group_depth(group: &ConditionGroup) -> usize {
        fn depth_clause(clause: &ConditionClause) -> usize {
            match &clause.group {
                Some(group) => 1 + WorkflowLoader::condition_group_depth(group),
                None => 1,
            }
        }

        let mut max_depth = 1;
        for clause in &group.conditions {
            max_depth = max_depth.max(depth_clause(clause));
        }
        max_depth
    }

    fn has_cycle(workflow: &WorkflowDefinition) -> bool {
        let mut graph: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        for step in &workflow.steps {
            let mut edges = Vec::new();
            if let Some(next) = step.next.as_deref() {
                edges.push(next);
            }
            if let Some(next) = step.on_success.as_deref() {
                edges.push(next);
            }
            if let Some(next) = step.on_failure.as_deref() {
                edges.push(next);
            }
            if step.kind == WorkflowStepKind::Switch {
                if let Some(cases) = step
                    .config
                    .get("cases")
                    .and_then(serde_json::Value::as_object)
                {
                    for target in cases.values().filter_map(serde_json::Value::as_str) {
                        edges.push(target);
                    }
                }
                if let Some(default_target) = step
                    .config
                    .get("default")
                    .and_then(serde_json::Value::as_str)
                {
                    edges.push(default_target);
                }
            }
            graph.insert(step.id.as_str(), edges);
        }

        fn dfs<'a>(
            node: &'a str,
            graph: &std::collections::HashMap<&'a str, Vec<&'a str>>,
            visiting: &mut std::collections::HashSet<&'a str>,
            visited: &mut std::collections::HashSet<&'a str>,
        ) -> bool {
            if visiting.contains(node) {
                return true;
            }
            if visited.contains(node) {
                return false;
            }

            visiting.insert(node);
            if let Some(neighbors) = graph.get(node) {
                for neighbor in neighbors {
                    if dfs(neighbor, graph, visiting, visited) {
                        return true;
                    }
                }
            }
            visiting.remove(node);
            visited.insert(node);
            false
        }

        let mut visiting = std::collections::HashSet::new();
        let mut visited = std::collections::HashSet::new();
        for node in graph.keys().copied() {
            if dfs(node, &graph, &mut visiting, &mut visited) {
                return true;
            }
        }

        false
    }

    fn resolve_dir(raw: &str, base: &Path) -> PathBuf {
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }

    fn discover_files(config: &WorkflowsConfig, base: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for dir in &config.directories {
            let root = Self::resolve_dir(dir, base);
            if !root.exists() || !root.is_dir() {
                continue;
            }

            let mut queue = VecDeque::new();
            queue.push_back((root, 0usize));
            while let Some((current, depth)) = queue.pop_front() {
                let entries = match std::fs::read_dir(&current) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if depth < config.max_discovery_depth {
                            queue.push_back((path, depth + 1));
                        }
                        continue;
                    }
                    if !path.is_file() {
                        continue;
                    }

                    let ext = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    if matches!(ext, "json" | "yaml" | "yml") {
                        files.push(path);
                    }
                }
            }
        }
        files
    }

    fn parse_file(path: &Path) -> Result<WorkflowDefinition> {
        let raw = std::fs::read_to_string(path).map_err(|err| {
            Error::Config(format!(
                "failed reading workflow file '{}': {err}",
                path.display()
            ))
        })?;

        let value = Self::parse_definition_value(path, &raw)?;
        Self::validate_against_schema(path, &value)?;
        serde_json::from_value::<WorkflowDefinition>(value).map_err(|err| {
            Error::Config(format!(
                "failed parsing workflow definition '{}': {err}",
                path.display()
            ))
        })
    }

    fn validate(
        workflow: &WorkflowDefinition,
        file: &Path,
        config: &WorkflowsConfig,
    ) -> Result<()> {
        if workflow.name.trim().is_empty() {
            return Err(Error::Validation(format!(
                "workflow '{}' must define non-empty name",
                file.display()
            )));
        }
        if workflow.steps.is_empty() {
            return Err(Error::Validation(format!(
                "workflow '{}' must define at least one step",
                workflow.name
            )));
        }

        if let Some(max_steps) = workflow.execution.max_steps_per_run {
            if max_steps == 0 {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.max_steps_per_run must be greater than 0",
                    workflow.name
                )));
            }
            if let Some(global_max_steps) = config.max_steps_per_run {
                if max_steps > global_max_steps {
                    return Err(Error::Validation(format!(
                        "workflow '{}' execution.max_steps_per_run {} exceeds global workflows.max_steps_per_run {}",
                        workflow.name, max_steps, global_max_steps
                    )));
                }
            }
        }
        if let Some(max_depth) = workflow.execution.max_recursion_depth {
            if max_depth == 0 {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.max_recursion_depth must be greater than 0",
                    workflow.name
                )));
            }
            if let Some(global_depth) = config.max_recursion_depth {
                if max_depth > global_depth {
                    return Err(Error::Validation(format!(
                        "workflow '{}' execution.max_recursion_depth {} exceeds global workflows.max_recursion_depth {}",
                        workflow.name, max_depth, global_depth
                    )));
                }
            }
        }
        if let Some(value) = workflow.execution.condition_group_max_depth {
            if value == 0 || value > config.condition_group_max_depth {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.condition_group_max_depth {} is invalid or exceeds global {}",
                    workflow.name, value, config.condition_group_max_depth
                )));
            }
        }
        if let Some(value) = workflow.execution.expression_max_length {
            if value == 0 || value > config.expression_max_length {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.expression_max_length {} is invalid or exceeds global {}",
                    workflow.name, value, config.expression_max_length
                )));
            }
        }
        if let Some(value) = workflow.execution.expression_max_depth {
            if value == 0 || value > config.expression_max_depth {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.expression_max_depth {} is invalid or exceeds global {}",
                    workflow.name, value, config.expression_max_depth
                )));
            }
        }
        if let Some(value) = workflow.execution.loop_default_max_iterations {
            if value == 0 {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.loop_default_max_iterations must be greater than 0",
                    workflow.name
                )));
            }
        }
        if let Some(value) = workflow.execution.loop_default_max_parallelism {
            if value == 0 {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.loop_default_max_parallelism must be greater than 0",
                    workflow.name
                )));
            }
        }
        if let Some(value) = workflow.execution.loop_hard_max_parallelism {
            if value == 0 || value > config.loop_hard_max_parallelism {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.loop_hard_max_parallelism {} is invalid or exceeds global {}",
                    workflow.name, value, config.loop_hard_max_parallelism
                )));
            }
        }
        if let (Some(default_parallel), Some(hard_parallel)) = (
            workflow.execution.loop_default_max_parallelism,
            workflow.execution.loop_hard_max_parallelism,
        ) {
            if default_parallel > hard_parallel {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.loop_default_max_parallelism cannot exceed execution.loop_hard_max_parallelism",
                    workflow.name
                )));
            }
        }
        if let Some(value) = workflow.execution.wait_default_poll_interval_ms {
            if value == 0 {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.wait_default_poll_interval_ms must be greater than 0",
                    workflow.name
                )));
            }
        }
        if let Some(value) = workflow.execution.wait_default_timeout_seconds {
            if value == 0 {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.wait_default_timeout_seconds must be greater than 0",
                    workflow.name
                )));
            }
        }
        if let Some(priority) = workflow.execution.switch_pattern_priority.as_deref() {
            if !matches!(priority, "exact_first" | "pattern_first") {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.switch_pattern_priority must be exact_first or pattern_first",
                    workflow.name
                )));
            }
        }
        if let Some(routing) = workflow.execution.continue_on_error_routing.as_deref() {
            if !matches!(routing, "next_first" | "on_failure_first") {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.continue_on_error_routing must be next_first or on_failure_first",
                    workflow.name
                )));
            }
        }
        if let Some(policy) = workflow.execution.execution_error_policy.as_deref() {
            if !matches!(policy, "abort" | "route_as_failure") {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.execution_error_policy must be abort or route_as_failure",
                    workflow.name
                )));
            }
        }
        if let Some(policy) = workflow.execution.timeout_error_policy.as_deref() {
            if !matches!(policy, "abort" | "route_as_failure") {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.timeout_error_policy must be abort or route_as_failure",
                    workflow.name
                )));
            }
        }
        if let Some(multiplier) = workflow.execution.default_retry_backoff_multiplier {
            if !(1.0..=10.0).contains(&multiplier) {
                return Err(Error::Validation(format!(
                    "workflow '{}' execution.default_retry_backoff_multiplier must be between 1.0 and 10.0",
                    workflow.name
                )));
            }
        }

        let effective_condition_depth = workflow
            .execution
            .condition_group_max_depth
            .unwrap_or(config.condition_group_max_depth);
        let effective_expression_max_length = workflow
            .execution
            .expression_max_length
            .unwrap_or(config.expression_max_length);
        let effective_expression_max_depth = workflow
            .execution
            .expression_max_depth
            .unwrap_or(config.expression_max_depth);
        let effective_loop_hard_parallelism = workflow
            .execution
            .loop_hard_max_parallelism
            .unwrap_or(config.loop_hard_max_parallelism);

        if let Some(max_steps) = config.max_steps_per_run {
            if workflow.steps.len() > max_steps {
                return Err(Error::Validation(format!(
                    "workflow '{}' has {} steps which exceeds max_steps_per_run ({max_steps})",
                    workflow.name,
                    workflow.steps.len()
                )));
            }
        }

        let mut ids = HashSet::new();
        for step in &workflow.steps {
            if step.id.trim().is_empty() {
                return Err(Error::Validation(format!(
                    "workflow '{}' contains a step with empty id",
                    workflow.name
                )));
            }
            if !ids.insert(step.id.clone()) {
                return Err(Error::Validation(format!(
                    "workflow '{}' contains duplicate step id '{}'",
                    workflow.name, step.id
                )));
            }
            if let Some(mode) = step
                .config
                .get("expression_error_mode")
                .and_then(|value| value.as_str())
            {
                if !matches!(mode, "strict" | "null" | "literal") {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' has unsupported expression_error_mode '{}'; expected strict|null|literal",
                        workflow.name, step.id, mode
                    )));
                }
            }

            if let Some(policy) = step
                .config
                .get("execution_error_policy")
                .and_then(|value| value.as_str())
            {
                if !matches!(policy, "abort" | "route_as_failure") {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' has unsupported execution_error_policy '{}'; expected abort|route_as_failure",
                        workflow.name, step.id, policy
                    )));
                }
            }

            if let Some(policy) = step
                .config
                .get("timeout_error_policy")
                .and_then(|value| value.as_str())
            {
                if !matches!(policy, "abort" | "route_as_failure") {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' has unsupported timeout_error_policy '{}'; expected abort|route_as_failure",
                        workflow.name, step.id, policy
                    )));
                }
            }

            if let Some(routing) = step
                .config
                .get("continue_on_error_routing")
                .and_then(|value| value.as_str())
            {
                if !matches!(routing, "next_first" | "on_failure_first") {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' has unsupported continue_on_error_routing '{}'; expected next_first|on_failure_first",
                        workflow.name, step.id, routing
                    )));
                }
            }

            if let Some(multiplier) = step
                .config
                .get("retry_backoff_multiplier")
                .and_then(|value| value.as_f64())
            {
                if !(1.0..=10.0).contains(&multiplier) {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' retry_backoff_multiplier must be between 1.0 and 10.0",
                        workflow.name, step.id
                    )));
                }
            }

            if let Some(max_length) = step
                .config
                .get("expression_max_length")
                .and_then(|value| value.as_u64())
            {
                if max_length == 0 || max_length as usize > effective_expression_max_length {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' expression_max_length {} is invalid or exceeds effective max {}",
                        workflow.name, step.id, max_length, effective_expression_max_length
                    )));
                }
            }

            if let Some(step_timeout_seconds) = step
                .config
                .get("step_timeout_seconds")
                .and_then(|value| value.as_u64())
            {
                if step_timeout_seconds == 0 {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' step_timeout_seconds must be greater than 0",
                        workflow.name, step.id
                    )));
                }
            }

            if let Some(max_depth) = step
                .config
                .get("expression_max_depth")
                .and_then(|value| value.as_u64())
            {
                if max_depth == 0 || max_depth as usize > effective_expression_max_depth {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' expression_max_depth {} is invalid or exceeds effective max {}",
                        workflow.name, step.id, max_depth, effective_expression_max_depth
                    )));
                }
            }

            if let Some(null_handling) = step
                .config
                .get("null_handling")
                .and_then(|value| value.as_str())
            {
                if !matches!(null_handling, "strict" | "lenient") {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' has unsupported null_handling '{}'; expected strict|lenient",
                        workflow.name, step.id, null_handling
                    )));
                }
            }

            if step.kind == WorkflowStepKind::Condition {
                let path_present = step
                    .config
                    .get("path")
                    .and_then(|value| value.as_str())
                    .is_some();
                let expression_present = step
                    .config
                    .get("expression")
                    .and_then(|value| value.as_str())
                    .is_some();
                let group_present = step.config.get("group").is_some();
                if !path_present && !expression_present && !group_present {
                    return Err(Error::Validation(format!(
                        "workflow '{}' condition step '{}' must define config.path, config.expression, or config.group",
                        workflow.name, step.id
                    )));
                }

                if let Some(group) = step.config.get("group") {
                    let group: ConditionGroup =
                        serde_json::from_value(group.clone()).map_err(|err| {
                            Error::Validation(format!(
                                "workflow '{}' condition step '{}' has invalid config.group: {err}",
                                workflow.name, step.id
                            ))
                        })?;
                    let depth = Self::condition_group_depth(&group);
                    if depth > effective_condition_depth {
                        return Err(Error::Validation(format!(
                            "workflow '{}' condition step '{}' exceeds max condition group nesting depth ({})",
                            workflow.name, step.id, effective_condition_depth
                        )));
                    }
                }
            }

            if step.kind == WorkflowStepKind::Wait {
                let has_duration = step
                    .config
                    .get("duration_seconds")
                    .and_then(|value| value.as_u64())
                    .is_some();
                let has_until = step
                    .config
                    .get("until_expression")
                    .and_then(|value| value.as_str())
                    .is_some();
                if !has_duration && !has_until {
                    return Err(Error::Validation(format!(
                        "workflow '{}' wait step '{}' must define duration_seconds or until_expression",
                        workflow.name, step.id
                    )));
                }

                if let Some(poll_interval_ms) = step
                    .config
                    .get("poll_interval_ms")
                    .and_then(|value| value.as_u64())
                {
                    if poll_interval_ms == 0 {
                        return Err(Error::Validation(format!(
                            "workflow '{}' wait step '{}' has invalid poll_interval_ms 0",
                            workflow.name, step.id
                        )));
                    }
                }

                if let Some(timeout_seconds) = step
                    .config
                    .get("timeout_seconds")
                    .and_then(|value| value.as_u64())
                {
                    if timeout_seconds == 0 {
                        return Err(Error::Validation(format!(
                            "workflow '{}' wait step '{}' has invalid timeout_seconds 0",
                            workflow.name, step.id
                        )));
                    }
                }
            }

            if step.kind == WorkflowStepKind::Switch {
                let has_cases = step
                    .config
                    .get("cases")
                    .and_then(|value| value.as_object())
                    .map(|cases| !cases.is_empty())
                    .unwrap_or(false);
                let has_pattern_cases = step
                    .config
                    .get("pattern_cases")
                    .and_then(|value| value.as_array())
                    .map(|cases| !cases.is_empty())
                    .unwrap_or(false);
                let has_default = step
                    .config
                    .get("default")
                    .and_then(|value| value.as_str())
                    .is_some();
                if !has_cases && !has_pattern_cases && !has_default && step.next.is_none() {
                    return Err(Error::Validation(format!(
                        "workflow '{}' switch step '{}' must define cases/pattern_cases/default/next target",
                        workflow.name, step.id
                    )));
                }

                if let Some(pattern_cases) = step
                    .config
                    .get("pattern_cases")
                    .and_then(|value| value.as_array())
                {
                    for (index, case) in pattern_cases.iter().enumerate() {
                        let case_obj = case.as_object().ok_or_else(|| {
                            Error::Validation(format!(
                                "workflow '{}' switch step '{}' pattern_cases[{}] must be an object",
                                workflow.name, step.id, index
                            ))
                        })?;

                        let pattern = case_obj
                            .get("pattern")
                            .and_then(|value| value.as_str())
                            .ok_or_else(|| {
                                Error::Validation(format!(
                                    "workflow '{}' switch step '{}' pattern_cases[{}] missing string 'pattern'",
                                    workflow.name, step.id, index
                                ))
                            })?;
                        let flags = case_obj
                            .get("flags")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");

                        let mut builder = RegexBuilder::new(pattern);
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
                                    return Err(Error::Validation(format!(
                                        "workflow '{}' switch step '{}' pattern_cases[{}] has unsupported regex flag '{}'",
                                        workflow.name, step.id, index, flag
                                    )));
                                }
                            }
                        }
                        builder.build().map_err(|err| {
                            Error::Validation(format!(
                                "workflow '{}' switch step '{}' pattern_cases[{}] invalid regex '{}': {err}",
                                workflow.name, step.id, index, pattern
                            ))
                        })?;

                        let _ = case_obj
                            .get("target")
                            .and_then(|value| value.as_str())
                            .ok_or_else(|| {
                                Error::Validation(format!(
                                    "workflow '{}' switch step '{}' pattern_cases[{}] missing string 'target'",
                                    workflow.name, step.id, index
                                ))
                            })?;
                    }
                }

                if let Some(priority) = step
                    .config
                    .get("pattern_priority")
                    .and_then(|value| value.as_str())
                {
                    if !matches!(priority, "exact_first" | "pattern_first") {
                        return Err(Error::Validation(format!(
                            "workflow '{}' switch step '{}' has unsupported pattern_priority '{}'; expected exact_first|pattern_first",
                            workflow.name, step.id, priority
                        )));
                    }
                }
            }

            if step.kind == WorkflowStepKind::Loop {
                let has_items = step
                    .config
                    .get("items")
                    .map(|value| !value.is_null())
                    .unwrap_or(false);
                if !has_items {
                    return Err(Error::Validation(format!(
                        "workflow '{}' loop step '{}' must define config.items",
                        workflow.name, step.id
                    )));
                }
                if let Some(max_iterations) = step
                    .config
                    .get("max_iterations")
                    .and_then(|value| value.as_u64())
                {
                    if max_iterations == 0 {
                        return Err(Error::Validation(format!(
                            "workflow '{}' loop step '{}' has invalid max_iterations 0",
                            workflow.name, step.id
                        )));
                    }
                }

                if let Some(max_parallelism) = step
                    .config
                    .get("max_parallelism")
                    .and_then(|value| value.as_u64())
                {
                    if max_parallelism == 0 {
                        return Err(Error::Validation(format!(
                            "workflow '{}' loop step '{}' has invalid max_parallelism 0",
                            workflow.name, step.id
                        )));
                    }
                    if max_parallelism > effective_loop_hard_parallelism {
                        return Err(Error::Validation(format!(
                            "workflow '{}' loop step '{}' has max_parallelism {} exceeding effective loop_hard_max_parallelism {}",
                            workflow.name, step.id, max_parallelism, effective_loop_hard_parallelism
                        )));
                    }
                }

                let parallel = step
                    .config
                    .get("parallel")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if parallel {
                    let has_result_expression = step
                        .config
                        .get("result_expression")
                        .and_then(|value| value.as_str())
                        .is_some();
                    if !has_result_expression {
                        return Err(Error::Validation(format!(
                            "workflow '{}' loop step '{}' enables parallel=true but has no result_expression",
                            workflow.name, step.id
                        )));
                    }
                }
            }

            if step.kind == WorkflowStepKind::Merge {
                let has_inputs = step
                    .config
                    .get("inputs")
                    .and_then(|value| value.as_object())
                    .map(|obj| !obj.is_empty())
                    .unwrap_or(false);
                if !has_inputs {
                    return Err(Error::Validation(format!(
                        "workflow '{}' merge step '{}' must define non-empty config.inputs object",
                        workflow.name, step.id
                    )));
                }

                if let Some(mode) = step.config.get("mode").and_then(|value| value.as_str()) {
                    if !matches!(mode, "merge" | "append" | "combine" | "multiplex") {
                        return Err(Error::Validation(format!(
                            "workflow '{}' merge step '{}' has unsupported mode '{}'; expected one of merge|append|combine|multiplex",
                            workflow.name, step.id, mode
                        )));
                    }
                }
            }
        }

        for step in &workflow.steps {
            for target in [&step.next, &step.on_success, &step.on_failure]
                .into_iter()
                .flatten()
            {
                if !ids.contains(target) {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' references unknown target step '{}'",
                        workflow.name, step.id, target
                    )));
                }
            }

            if step.kind == WorkflowStepKind::Switch {
                if let Some(cases) = step
                    .config
                    .get("cases")
                    .and_then(serde_json::Value::as_object)
                {
                    for target in cases.values().filter_map(serde_json::Value::as_str) {
                        if !ids.contains(target) {
                            return Err(Error::Validation(format!(
                                "workflow '{}' switch step '{}' references unknown case target step '{}'",
                                workflow.name, step.id, target
                            )));
                        }
                    }
                }
                if let Some(default_target) = step
                    .config
                    .get("default")
                    .and_then(serde_json::Value::as_str)
                {
                    if !ids.contains(default_target) {
                        return Err(Error::Validation(format!(
                            "workflow '{}' switch step '{}' references unknown default step '{}'",
                            workflow.name, step.id, default_target
                        )));
                    }
                }

                if let Some(pattern_cases) = step
                    .config
                    .get("pattern_cases")
                    .and_then(serde_json::Value::as_array)
                {
                    for case in pattern_cases {
                        if let Some(target) = case.get("target").and_then(serde_json::Value::as_str)
                        {
                            if !ids.contains(target) {
                                return Err(Error::Validation(format!(
                                    "workflow '{}' switch step '{}' references unknown pattern target step '{}'",
                                    workflow.name, step.id, target
                                )));
                            }
                        }
                    }
                }
            }
        }

        if Self::has_cycle(workflow) {
            return Err(Error::Validation(format!(
                "workflow '{}' contains a circular step reference graph",
                workflow.name
            )));
        }

        if workflow.entrypoints.is_empty() {
            return Err(Error::Validation(format!(
                "workflow '{}' must define at least one entrypoint",
                workflow.name
            )));
        }

        for (name, entrypoint) in &workflow.entrypoints {
            if name.trim().is_empty() {
                return Err(Error::Validation(format!(
                    "workflow '{}' has empty entrypoint name",
                    workflow.name
                )));
            }
            if entrypoint.step.trim().is_empty() || !ids.contains(&entrypoint.step) {
                return Err(Error::Validation(format!(
                    "workflow '{}' entrypoint '{}' references unknown step '{}'",
                    workflow.name, name, entrypoint.step
                )));
            }
        }

        Ok(())
    }

    pub fn load(config: &WorkflowsConfig, work_dir: &Path) -> Result<WorkflowRegistry> {
        let mut registry = WorkflowRegistry::new();
        let files = Self::discover_files(config, work_dir);
        for file in files {
            let workflow = Self::parse_file(&file)?;
            Self::validate(&workflow, &file, config)?;
            registry.register(workflow);
        }
        Ok(registry)
    }
}
