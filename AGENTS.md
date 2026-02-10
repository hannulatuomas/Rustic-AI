# AGENTS.md (Repository Guide for Coding Agents)

This file defines how agentic coding assistants should operate in this repository.

## Repository Overview
- Workspace: Rust monorepo (edition 2021).
- Core crate: `rustic-ai-core` (engine/library, UI-agnostic).
- Frontend crate: `frontend/rustic-ai-cli` (CLI consumer).
- Rule: keep product/runtime logic in core; keep UX/CLI behavior in frontend.
- Keep implementation complete: no placeholders, no fake integrations.

## Required Reading Before Non-Trivial Changes
- `docs/DESIGN_GUIDE.md`
- `docs/DECISIONS.md`
- `docs/initial-planning/big-picture.md`
- `docs/initial-planning/integration-plan.md`
- `docs/initial-planning/REQUIREMENTS.md`
- `docs/initial-planning/tools.md`
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
2. Plan integration impact (config, runtime, storage, providers, tools, docs).
3. Implement full behavior (no scaffold-only changes).
4. Validate build/lint/tests for touched scope.
5. Update docs + `TODO.md` in the same change.

## Build, Format, Lint, and Test Commands
Run all commands from repository root.

### Environment
- Ensure Cargo is available:
  - `export PATH="$HOME/.cargo/bin:$PATH"`

### Build
- Workspace debug build: `cargo build --workspace`
- Workspace release build: `cargo build --workspace --release`
- Core crate only: `cargo build -p rustic-ai-core`
- CLI crate only: `cargo build -p rustic-ai-cli`

### Format
- Check formatting only: `cargo fmt --all -- --check`
- Apply formatting: `cargo fmt --all`

### Lint (Clippy)
- Strict workspace lint:
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Strict core crate lint:
  - `cargo clippy -p rustic-ai-core --all-targets --all-features -- -D warnings`
- Strict CLI crate lint:
  - `cargo clippy -p rustic-ai-cli --all-targets --all-features -- -D warnings`

### Test (Full and Scoped)
- Full workspace tests:
  - `cargo test --workspace --all-features`
- One crate tests:
  - `cargo test -p rustic-ai-core --all-features`
  - `cargo test -p rustic-ai-cli --all-features`

### Single-Test Commands (Important)
- Run by test name substring:
  - `cargo test -p rustic-ai-core accepts_minimal_valid_config -- --nocapture`
- Run exact fully-qualified test path:
  - `cargo test -p rustic-ai-core config::validation::tests::accepts_minimal_valid_config -- --exact --nocapture`
- Run one module subset:
  - `cargo test -p rustic-ai-core config::validation -- --nocapture`
- Run a single loader test by exact name:
  - `cargo test -p rustic-ai-core config::loader::tests::merge_prefers_non_empty_override_vectors -- --exact --nocapture`
- Run one CLI test by exact path:
  - `cargo test -p rustic-ai-cli cli::tests::parses_config_flag -- --exact --nocapture`
- Run ignored tests only:
  - `cargo test -p rustic-ai-core -- --ignored --nocapture`

## Runtime and Config Validation Commands
- Validate config against schema and runtime rules:
  - `cargo run -p rustic-ai-cli -- --config config.json validate-config --schema docs/config.schema.json`
- Strict validation mode (recommended):
  - `cargo run -p rustic-ai-cli -- --config config.json validate-config --strict`
- List auth capability matrix from config:
  - `cargo run -p rustic-ai-cli -- --config config.json auth methods`

## Key Configuration Files
- Example config: `config.example.json`
- Runtime default config: `config.json`
- Main JSON schema: `docs/config.schema.json`
- CLI envelope schema: `docs/config.cli-output.schema.json`

Design constraints:
- Do not add provider-specific hardcoded fallback values.
- Require explicit provider model/base URL/credential env var names where needed.
- Enforce provider/auth-mode compatibility in validation.

## Code Style Guidelines (Rust)

### Formatting and File Organization
- Treat `rustfmt` output as canonical.
- Keep files cohesive by module responsibility.
- Keep functions focused; extract helpers for deeply nested logic.
- Add comments only for non-obvious rationale.

### Imports
- Group imports in this order with blank lines:
  1) `std::...`
  2) external crates
  3) `crate::...` or `super::...`
- Avoid glob imports (`*`).
- Remove unused imports instead of allowing warnings.

### Naming Conventions
- Modules/files/functions/variables: `snake_case`
- Structs/enums/traits: `PascalCase`
- Constants/statics: `SCREAMING_SNAKE_CASE`
- Name booleans to read clearly at call site (`is_*`, `has_*`, `should_*`).

### Types and API Boundaries
- Prefer explicit typed structs/enums over stringly-typed maps.
- Minimize public surface area (`pub` only where required).
- Keep provider/tool-specific details behind module boundaries.
- Prefer constructors and validated config objects over mutable public fields.

### Error Handling
- No `unwrap()` / `expect()` in runtime paths.
- Prefer typed errors (`thiserror`) and propagate with context.
- Error messages should state what failed and where.
- In tests, `expect()` is acceptable when message improves diagnosis.

### Async, Concurrency, and I/O
- Use Tokio-native async APIs.
- Avoid blocking work in async contexts.
- Do not hold locks across `.await` points.
- Use timeouts/cancellation for long-running operations.

### Logging and Security
- Use `tracing` for diagnostics.
- Never log secrets, tokens, API keys, or sensitive payloads.
- Never persist raw credentials in session/history stores.
- Redact sensitive values in user-facing and debug output.

### Testing Expectations
- Add/adjust unit tests when changing behavior.
- Prefer deterministic tests (no external network dependencies).
- Keep test fixtures minimal and explicit.
- Validate both success and failure paths for config/provider/tool logic.

## Repository-Specific Practices
- Update `TODO.md` when starting/completing/re-scoping non-trivial work.
- Update `docs/DECISIONS.md` for architecture or boundary changes.
- Keep `README.md` and config docs aligned with actual behavior.
- Preserve core/frontend separation in all new code.

## Definition of Done
- Touched scope compiles.
- `cargo fmt --all` passes.
- Clippy is clean for touched scope (`-D warnings`).
- Relevant tests pass, including targeted single-test runs where appropriate.
- Docs and `TODO.md` updated in the same change.
