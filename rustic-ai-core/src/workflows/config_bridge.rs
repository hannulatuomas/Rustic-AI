use std::path::PathBuf;

use crate::config::schema::WorkflowsConfig;

use super::executor::WorkflowExecutorConfig;

pub fn build_workflow_executor_config(
    workflows_config: &WorkflowsConfig,
    working_directory: PathBuf,
) -> WorkflowExecutorConfig {
    WorkflowExecutorConfig {
        max_recursion_depth: workflows_config.max_recursion_depth,
        max_steps_per_run: workflows_config.max_steps_per_run,
        working_directory,
        default_timeout_seconds: workflows_config.default_timeout_seconds,
        compatibility_preset: workflows_config.compatibility_preset,
        switch_case_sensitive_default: workflows_config.switch_case_sensitive_default,
        switch_pattern_priority: workflows_config.switch_pattern_priority.clone(),
        loop_continue_on_iteration_error_default: workflows_config
            .loop_continue_on_iteration_error_default,
        wait_timeout_succeeds: workflows_config.wait_timeout_succeeds,
        condition_missing_path_as_false: workflows_config.condition_missing_path_as_false,
        default_continue_on_error: workflows_config.default_continue_on_error,
        continue_on_error_routing: workflows_config.continue_on_error_routing.clone(),
        execution_error_policy: workflows_config.execution_error_policy.clone(),
        timeout_error_policy: workflows_config.timeout_error_policy.clone(),
        default_retry_count: workflows_config.default_retry_count,
        default_retry_backoff_ms: workflows_config.default_retry_backoff_ms,
        default_retry_backoff_multiplier: workflows_config.default_retry_backoff_multiplier,
        default_retry_backoff_max_ms: workflows_config.default_retry_backoff_max_ms,
        condition_group_max_depth: workflows_config.condition_group_max_depth,
        expression_max_length: workflows_config.expression_max_length,
        expression_max_depth: workflows_config.expression_max_depth,
        loop_default_max_iterations: workflows_config.loop_default_max_iterations,
        loop_default_max_parallelism: workflows_config.loop_default_max_parallelism,
        loop_hard_max_parallelism: workflows_config.loop_hard_max_parallelism,
        wait_default_poll_interval_ms: workflows_config.wait_default_poll_interval_ms,
        wait_default_timeout_seconds: workflows_config.wait_default_timeout_seconds,
    }
}
