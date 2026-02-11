# Workflow Compatibility Presets

This document describes the runtime behavior presets used by `workflows.compatibility_preset`.

Preset behavior is applied first, then overridden by:
1. Global workflow toggles in `config.json` (`workflows.*`), then
2. Per-workflow `execution.*` overrides, then
3. Per-step `config.*` overrides where supported.

## Preset Matrix

| Behavior | rustic | open_code | claude_code | n8n |
|---|---|---|---|---|
| `null_handling` default | strict | strict | strict | lenient |
| `switch_case_sensitive_default` | true | true | true | false |
| `switch_pattern_priority` | exact_first | exact_first | exact_first | pattern_first |
| `loop_continue_on_iteration_error_default` | false | false | false | true |
| `wait_timeout_succeeds` | false | false | false | true |
| `condition_missing_path_as_false` | false | false | false | true |
| `default_continue_on_error` | false | false | false | true |
| `continue_on_error_routing` | next_first | next_first | next_first | next_first |
| `execution_error_policy` | abort | abort | abort | route_as_failure |
| `default_retry_count` | 0 | 0 | 0 | 2 |
| `default_retry_backoff_ms` | 250 | 250 | 250 | 500 |
| `default_retry_backoff_multiplier` | 2.0 | 2.0 | 2.0 | 2.0 |
| `default_retry_backoff_max_ms` | 10000 | 10000 | 10000 | 30000 |

## Runtime Toggles

Global toggles live under `workflows` in main config:

- `switch_case_sensitive_default`
- `switch_pattern_priority` (`exact_first` or `pattern_first`)
- `loop_continue_on_iteration_error_default`
- `wait_timeout_succeeds`
- `condition_missing_path_as_false`
- `default_continue_on_error`
- `continue_on_error_routing` (`next_first` or `on_failure_first`)
- `execution_error_policy` (`abort` or `route_as_failure`)
- `default_retry_count`
- `default_retry_backoff_ms`
- `default_retry_backoff_multiplier`
- `default_retry_backoff_max_ms`

Per-workflow overrides use the same names under workflow `execution`.

Per-step overrides:

- `switch` step:
  - `config.case_sensitive`
  - `config.pattern_priority`
  - `config.retry_on_no_match`
- `condition` step:
  - `config.retry_on_false`
- `loop` step:
  - `config.continue_on_iteration_error`
- `wait` step:
  - `config.timeout_succeeds`
- all step kinds:
  - `config.retry_count`
  - `config.retry_backoff_ms`
  - `config.retry_backoff_multiplier`
  - `config.retry_backoff_max_ms`
  - `config.execution_error_policy`
  - `config.continue_on_error`
  - `config.continue_on_error_routing`
- all expression-capable steps:
  - `config.expression_error_mode`
  - `config.expression_max_length`
  - `config.expression_max_depth`
  - `config.null_handling`

## Reference Examples

See:

- `docs/workflow-examples/opencode.parallel-checks.workflow.json`
- `docs/workflow-examples/opencode.multi-agent-orchestration.workflow.json`
- `docs/workflow-examples/claude-code.review-routing.workflow.json`
- `docs/workflow-examples/claude-code.nested-review-rework.workflow.json`
- `docs/workflow-examples/claude-code.rework-subflow.workflow.json`
- `docs/workflow-examples/n8n.lenient-fanout.workflow.json`
- `docs/workflow-examples/n8n.webhook-retry-pipeline.workflow.json`
- `docs/workflow-examples/n8n.webhook-signature-gate.workflow.json`
- `docs/workflow-examples/n8n.webhook-idempotent-upsert.workflow.json`

Migration guidance:

- `docs/workflow-preset-migration.md`
- `docs/workflow-webhook-parity.md`
