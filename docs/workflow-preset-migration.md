# Workflow Preset Migration Guide

This guide helps migrate existing workflow definitions to the configurable preset model.

## Migration Goals

- Make runtime behavior explicit and reproducible across environments.
- Adopt preset defaults (`rustic`, `open_code`, `claude_code`, `n8n`) without breaking existing flows.
- Move ad-hoc error handling and retry logic into standardized workflow configuration.

## Migration Strategy

1. Pick a baseline preset in global config:
   - `workflows.compatibility_preset = rustic|open_code|claude_code|n8n`
2. Validate current behavior under that preset (dry-run environment).
3. Add per-workflow `execution` overrides only where behavior must differ.
4. Add per-step overrides for exceptional steps only.
5. Remove legacy inline branching/retry hacks once equivalent config is active.

## Mapping Checklist

Use this checklist per workflow:

- Null behavior:
  - Set `execution.null_handling` if workflow differs from preset.
  - For specific steps, set `config.null_handling`.
- Switch behavior:
  - Set `execution.switch_case_sensitive_default` / `execution.switch_pattern_priority`.
  - Override via `config.case_sensitive` / `config.pattern_priority` per switch step.
- Continue-on-error:
  - Set `execution.default_continue_on_error` and `execution.continue_on_error_routing`.
  - Override per step via `config.continue_on_error` and `config.continue_on_error_routing`.
- Error policy:
  - Set `execution.execution_error_policy` (`abort` or `route_as_failure`).
  - Override with step-level `config.execution_error_policy` for isolated routes.
- Retry/backoff:
  - Set workflow defaults:
    - `execution.default_retry_count`
    - `execution.default_retry_backoff_ms`
    - `execution.default_retry_backoff_multiplier`
    - `execution.default_retry_backoff_max_ms`
  - Override per step only where required.
- Safety limits:
  - Set `execution.max_steps_per_run`, `execution.max_recursion_depth` as needed.
  - Keep within global limits from `workflows.*` in top-level config.

## Recommended Rollout

- Phase 1 (observe): set preset only, no overrides.
- Phase 2 (stabilize): add workflow-level overrides for outliers.
- Phase 3 (optimize): add step-level overrides for known hot spots.
- Phase 4 (cleanup): remove old workaround logic and simplify definitions.

## Validation Commands

Run from repository root:

```bash
cargo run -p rustic-ai-cli -- --config config.json validate-config --strict
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace
```

## Reference Examples

- OpenCode style:
  - `docs/workflow-examples/opencode.parallel-checks.workflow.json`
  - `docs/workflow-examples/opencode.multi-agent-orchestration.workflow.json`
- Claude-code style:
  - `docs/workflow-examples/claude-code.review-routing.workflow.json`
  - `docs/workflow-examples/claude-code.nested-review-rework.workflow.json`
  - `docs/workflow-examples/claude-code.rework-subflow.workflow.json`
- n8n style:
  - `docs/workflow-examples/n8n.lenient-fanout.workflow.json`
  - `docs/workflow-examples/n8n.webhook-retry-pipeline.workflow.json`
