# Webhook Parity Guide (n8n-style)

This guide documents practical parity patterns for webhook workflows, focused on:

- Signature verification
- Idempotency/deduplication
- Branch/failure behavior compatible with event-driven pipelines

Use this with `workflows.compatibility_preset = n8n` for lenient/fanout defaults, then tighten per-workflow where needed.

## Signature Verification Pattern

Recommended flow:

1. Receive webhook payload and headers.
2. Verify signature in an explicit `tool` step.
3. Route via `switch` (`true` -> process, `false` -> reject).
4. Emit structured reject payload for observability.

Key points:

- Keep verification logic isolated in one step.
- Treat verification failure as a normal routed branch when desired (`execution_error_policy = route_as_failure`).
- Never continue into processing path without explicit verify pass.

## Idempotency Pattern

Recommended flow:

1. Extract idempotency key from headers/body.
2. Check store/cache for prior key.
3. Route existing keys to fast-return path.
4. Process only first-seen events.
5. Persist result with idempotency key.

Key points:

- Use stable key derivation (`source + endpoint + external_event_id`).
- Prefer deterministic response replay from stored result for duplicates.
- Include dedupe metadata in final payload (`is_duplicate`, `idempotency_key`).

## Suggested Config

Global (`config.json`):

- `workflows.compatibility_preset: "n8n"`
- `workflows.execution_error_policy: "route_as_failure"`
- `workflows.default_continue_on_error: true`

Per webhook workflow (`execution`):

- tighter `max_steps_per_run`
- explicit `default_retry_count` and backoff values
- explicit `null_handling` for edge payloads

## Reference Examples

- `docs/workflow-examples/n8n.webhook-signature-gate.workflow.json`
- `docs/workflow-examples/n8n.webhook-idempotent-upsert.workflow.json`
- `docs/workflow-examples/n8n.webhook-retry-pipeline.workflow.json`
