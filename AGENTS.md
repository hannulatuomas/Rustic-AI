# AGENTS.md (Repository Guide for Coding Agents)

This file defines how agentic coding assistants should operate in this repository.

## Repository Overview
- Workspace: Rust monorepo.
- Core crate: `rustic-ai-core` (engine/library, UI-agnostic).
- Frontend crate: `frontend/rustic-ai-cli` (CLI consumer).
- Rule: keep product/runtime logic in core; keep UX/CLI behavior in frontend.

## Required Reading Before Non-Trivial Changes
- `docs/DESIGN_GUIDE.md`
- `docs/DECISIONS.md`
- `docs/initial-planning/big-picture.md`
- `docs/initial-planning/integration-plan.md`
- `docs/initial-planning/REQUIREMENTS.md`
- `TODO.md`

## Cursor/Copilot Rules Status
Checked paths:
- `.cursor/rules/`
- `.cursorrules`
- `.github/copilot-instructions.md`

Current status: none of these rule files exist in this repository.

If they are added later, treat them as hard constraints and update this file.

## Delivery Workflow (Use This Sequence)
1. Read relevant docs and touched modules.
2. Plan integration impact (config, runtime, storage, provider, docs).
3. Implement complete behavior (no scaffold placeholders).
4. Validate build/lint/tests for touched scope.
5. Update docs + `TODO.md` in the same change.

## Build, Format, Lint, Test Commands
Run from repository root.

### Environment
- Ensure cargo path if needed:
  - `export PATH="$HOME/.cargo/bin:$PATH"`

### Build
- Workspace build: `cargo build --workspace`
- Release build: `cargo build --workspace --release`
- Core only: `cargo build -p rustic-ai-core`
- CLI only: `cargo build -p rustic-ai-cli`

### Format
- Check formatting: `cargo fmt --all -- --check`
- Apply formatting: `cargo fmt --all`

### Lint
- Strict workspace clippy:
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Strict single-crate clippy:
  - `cargo clippy -p rustic-ai-core --all-targets --all-features -- -D warnings`

### Test
- Full workspace tests: `cargo test --workspace --all-features`
- One crate: `cargo test -p rustic-ai-core --all-features`

### Single-Test (Important)
- Name substring:
  - `cargo test -p rustic-ai-core my_test_name -- --nocapture`
- Exact test path:
  - `cargo test -p rustic-ai-core config::validation::tests::accepts_minimal_valid_config -- --exact --nocapture`
- Module subset:
  - `cargo test -p rustic-ai-core config::validation -- --nocapture`
- Single CLI test:
  - `cargo test -p rustic-ai-cli cli::tests::parses_config_flag -- --exact --nocapture`
- Ignored tests:
  - `cargo test -p rustic-ai-core -- --ignored --nocapture`

## Config Validation Commands
- Schema + runtime validation:
  - `cargo run -p rustic-ai-cli -- --config config.json validate-config --schema docs/config.schema.json`
- Strict mode (recommended in CI):
  - `cargo run -p rustic-ai-cli -- --config config.json validate-config --strict`

## Configuration Files
- Example config: `config.example.json`
- Runtime default config path: `config.json`
- Schema: `docs/config.schema.json`

Design rule:
- Do not introduce hardcoded provider-specific fallback values.
- Require explicit configuration for provider model, base URL, and credential env var names.

## Code Style Guidelines (Rust)

### Formatting and Structure
- Use rustfmt output as source of truth.
- Keep functions focused and split long blocks into helpers.
- Add comments only for non-obvious rationale.

### Imports
- Group imports in order with blank lines:
  1) `std::...`
  2) external crates
  3) `crate::...` / `super::...`
- Avoid glob imports.
- Keep imports minimal and used.

### Naming
- Modules/files/functions/variables: `snake_case`
- Structs/enums/traits: `PascalCase`
- Constants/statics: `SCREAMING_SNAKE_CASE`
- Prefer precise names over abbreviations.

### Types and API Boundaries
- Prefer typed structs/enums to stringly-typed contracts.
- Keep public API minimal.
- Avoid leaking backend/provider internals across module boundaries.
- Prefer constructor methods over mutable public fields.

### Error Handling
- No `unwrap()` / `expect()` in runtime paths.
- Use crate error types (`thiserror`) and propagate with context.
- Error messages should state what failed and where.

### Async/Concurrency
- Use Tokio-native async patterns.
- Avoid blocking calls in async execution paths.
- Do not hold locks across `.await`.
- Support cancellation/timeouts for long-running operations.

### Logging/Security
- Use `tracing` for runtime diagnostics.
- Never log credentials, secrets, tokens, or sensitive payloads.
- Never persist secrets in session/history stores.

## Repository-Specific Practices
- Update `TODO.md` when completing or re-scoping tasks.
- Update `docs/DECISIONS.md` for architecture/config boundary changes.
- Keep `README.md` and config docs in sync with behavior.

## Definition of Done
- Compiles for touched scope.
- `cargo fmt --all` clean.
- Clippy clean for touched scope.
- Relevant tests pass.
- Docs and TODO updated together with code.
