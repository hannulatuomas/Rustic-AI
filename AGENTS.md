# AGENTS.md

Operational guide for agentic coding assistants working in this repository.

## 1) Repository Snapshot

- Rust workspace (edition 2021), resolver 2.
- Core crate: `rustic-ai-core` (runtime, providers, tools, workflows, permissions, storage, learning, indexing/RAG).
- Frontend crate: `frontend/rustic-ai-cli` (CLI commands, REPL, rendering).
- Architectural rule: keep business/runtime behavior in core; keep CLI UX in frontend.
- Current state: large feature surface, low automated test coverage; prioritize safe, minimal, verifiable changes.

## 2) Mandatory Context Files

Read these before non-trivial work:

- `README.md`
- `TODO.md`
- `docs/DESIGN_GUIDE.md`
- `docs/DECISIONS.md`
- `docs/comprehensive-state-analysis.md`
- `docs/initial-planning/REQUIREMENTS.md`
- `docs/initial-planning/tools.md`

If scope touches workflows/agents/retrieval, also read relevant module docs and adjacent source files.

## 3) Cursor/Copilot Rules

Checked locations:

- `.cursor/rules/`
- `.cursorrules`
- `.github/copilot-instructions.md`

Status at time of writing: none of these files exist.

If any are added later, treat them as hard constraints and update this file in the same change.

## 4) Build, Lint, and Test Commands

Run from repo root: `/home/debian/Github/Rustic-AI`.

Environment:

- `export PATH="$HOME/.cargo/bin:$PATH"`

Build:

- Workspace debug: `cargo build --workspace`
- Workspace release: `cargo build --workspace --release`
- Core only: `cargo build -p rustic-ai-core`
- CLI only: `cargo build -p rustic-ai-cli`

Format:

- Check only: `cargo fmt --all -- --check`
- Apply: `cargo fmt --all`

Lint (strict):

- Workspace: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Core only: `cargo clippy -p rustic-ai-core --all-targets --all-features -- -D warnings`
- CLI only: `cargo clippy -p rustic-ai-cli --all-targets --all-features -- -D warnings`

Tests:

- Full workspace: `cargo test --workspace --all-features`
- Per crate: `cargo test -p rustic-ai-core --all-features`
- Per crate: `cargo test -p rustic-ai-cli --all-features`

Single-test execution (important patterns):

- By substring: `cargo test -p rustic-ai-core <test_name_substring> -- --nocapture`
- Exact name/path: `cargo test -p rustic-ai-core <module::tests::test_name> -- --exact --nocapture`
- Module subset: `cargo test -p rustic-ai-core <module::tests> -- --nocapture`
- Ignored only: `cargo test -p rustic-ai-core -- --ignored --nocapture`
- Stop on first failure: `cargo test -p rustic-ai-core <pattern> -- --nocapture --test-threads=1`

Note: this repo may have sparse tests in some modules; if no tests match, add focused tests with behavior changes.

## 5) CLI/Runtime Validation Commands

- Strict config validation:
  - `cargo run -p rustic-ai-cli -- --config config.json validate-config --strict`
- Schema-based validation:
  - `cargo run -p rustic-ai-cli -- --config config.json validate-config --schema docs/config.schema.json`
- Auth capability matrix:
  - `cargo run -p rustic-ai-cli -- --config config.json auth methods`
- Index diagnostics:
  - `cargo run -p rustic-ai-cli -- --config config.json index status`

## 6) Code Style and Engineering Rules

### Formatting and Layout

- `rustfmt` output is canonical.
- Keep functions focused; split large branches into private helpers.
- Prefer cohesive modules over cross-cutting utility sprawl.
- Add comments only for non-obvious rationale or invariants.

### Imports

- Group imports in this order with blank lines:
  1. `std::...`
  2. third-party crates
  3. `crate::...` / `super::...`
- Avoid wildcard imports.
- Remove unused imports instead of allowing warnings.

### Naming

- `snake_case`: modules, files, functions, variables.
- `PascalCase`: structs, enums, traits.
- `SCREAMING_SNAKE_CASE`: constants/statics.
- Prefer intention-revealing names (`is_*`, `has_*`, `should_*`).

### Types and API Boundaries

- Prefer typed structs/enums over untyped JSON maps where practical.
- Keep `pub` surface minimal; default to private.
- Keep provider/tool internals behind module boundaries.
- Validate config at boundaries; avoid hidden fallbacks.

### Error Handling

- Do not use `unwrap()`/`expect()` in runtime paths.
- Use typed errors (`thiserror`) and propagate with context.
- Error messages should include failing operation and relevant identifier/path.
- `expect()` is acceptable in tests for clear diagnostics.

### Async and Concurrency

- Use Tokio-native async APIs.
- Avoid blocking calls in async contexts.
- Do not hold locks across `.await`.
- Use cancellation and timeout controls for long-running operations.

### Logging and Security

- Use `tracing` for structured diagnostics.
- Never log secrets, tokens, API keys, or raw credentials.
- Redact sensitive values in errors and events.
- Preserve permission checks when adding/changing tools.

## 7) Change Management Rules

- For non-trivial work, update `TODO.md` in the same change.
- For architecture/boundary decisions, update `docs/DECISIONS.md`.
- Keep `README.md` aligned with real implementation status.
- Do not ship placeholder/stub behavior labeled as complete.
- Do not revert user-authored unrelated changes.

## 8) Definition of Done

- Scope compiles for touched crates.
- `cargo fmt --all` passes.
- Clippy passes with `-D warnings` for touched scope (or workspace for broad changes).
- Relevant tests pass; run focused single-test commands when available.
- Docs (`README.md`, `TODO.md`, `DECISIONS.md`) updated when behavior/scope changed.

## 9) Practical Workflow for Agents

1. Inspect impacted modules and nearby patterns.
2. Implement minimal, complete change (no speculative rewrites).
3. Run format/build/lint.
4. Run the smallest useful test scope, then widen as needed.
5. Update docs and roadmap files.
6. Summarize what changed, how to verify, and what remains.
