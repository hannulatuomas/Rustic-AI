# AGENTS.md (Repository Guide for Coding Agents)

This repository is Rustic-AI. Follow this guide for all non-trivial changes.

## Repository Intent
- Rust workspace with two crates:
  - `rustic-ai-core` (UI-agnostic engine library)
  - `frontend/rustic-ai-cli` (CLI frontend consumer)
- Keep product logic in `rustic-ai-core`.
- Keep interface-specific behavior in `frontend/`.
- Prefer extending existing modules over introducing parallel abstractions.

## Required Reading Before Changes
- `docs/DESIGN_GUIDE.md` (required workflow and definition of done)
- `docs/DECISIONS.md` (ADR-lite architecture history)
- `docs/initial-planning/big-picture.md` (north star)
- `docs/initial-planning/integration-plan.md` (subsystem boundaries and flow)
- `docs/initial-planning/REQUIREMENTS.md` (capability + quality baseline)
- `docs/initial-planning/tools.md` (planned tool inventory)

## Cursor/Copilot Rules
No extra rules were found in:
- `.cursor/rules/`
- `.cursorrules`
- `.github/copilot-instructions.md`

If any of these files are later added, treat them as additional constraints and update this guide.

## Delivery Workflow (Quality Gate)
For non-trivial tasks, always execute this sequence:
1. Plan: inspect current code, docs, and integration impacts.
2. Implement: complete behavior end-to-end (no placeholder logic).
3. Validate: run build/lint/tests for touched scope.
4. Fix correctly: no warning suppression or shortcut workarounds.
5. Re-check architecture fit versus big-picture and integration docs.
6. Update TODO: update `TODO.md` so completed tasks are marked done and next tasks are accurate.

## Build / Format / Lint / Test
Run from repository root.

### Environment
- If `cargo` is not available, prepend PATH:
  - `export PATH="$HOME/.cargo/bin:$PATH"`

### Build Commands
- Build all crates: `cargo build --workspace`
- Build release: `cargo build --workspace --release`
- Build core only: `cargo build -p rustic-ai-core`
- Build CLI only: `cargo build -p rustic-ai-cli`

### Format Commands
- Check formatting: `cargo fmt --all -- --check`
- Apply formatting: `cargo fmt --all`

### Lint Commands
- Strict clippy (workspace):
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Strict clippy (single crate):
  - `cargo clippy -p rustic-ai-core --all-targets --all-features -- -D warnings`

### Test Commands
- Run all tests: `cargo test --workspace --all-features`
- Run one crate: `cargo test -p rustic-ai-core --all-features`

### Single-Test Commands (Important)
- By substring match:
  - `cargo test -p rustic-ai-core my_test_name -- --nocapture`
- Exact full test path:
  - `cargo test -p rustic-ai-core agents::memory::tests::evicts_old_messages -- --exact --nocapture`
- Module/file subset via module name:
  - `cargo test -p rustic-ai-core memory -- --nocapture`
- Single test in CLI crate:
  - `cargo test -p rustic-ai-cli cli::tests::parses_config_flag -- --exact --nocapture`
- Ignored tests:
  - `cargo test -p rustic-ai-core -- --ignored --nocapture`

### Docs
- Build docs: `cargo doc --workspace --no-deps`

## Code Style Guidelines (Rust)

### Formatting
- Use rustfmt defaults; do not fight formatter output.
- Keep functions focused and readable; extract helpers for long blocks.
- Avoid noisy comments; add comments only for non-obvious rationale.

### Imports
- Group imports in this order with blank lines:
  1) `std::...`
  2) external crates
  3) `crate::...` / `super::...`
- Prefer explicit imports over glob imports.
- Keep `use` lists small, sorted, and used.

### Naming
- Files/modules: `snake_case`
- Functions/variables: `snake_case`
- Types/traits/enums: `PascalCase`
- Constants/statics: `SCREAMING_SNAKE_CASE`
- Use descriptive names; avoid ambiguous abbreviations.

### Types and APIs
- Prefer typed structs/enums over stringly-typed values.
- Use newtypes for meaningful IDs when that improves correctness.
- Keep public APIs minimal and intentional.
- Favor constructors/builders over public mutable fields.
- Do not leak storage/provider implementation types across module boundaries.

### Error Handling
- Do not use `unwrap()` / `expect()` in runtime paths.
- Use typed errors (`thiserror`) for library-facing error surfaces.
- Add context at boundaries (storage, provider, tools, network).
- Error messages should explain what failed and where.

### Async and Concurrency
- Use Tokio primitives; avoid blocking calls on async executors.
- Prefer message passing to shared mutable state.
- Never hold locks across `.await`.
- Long-running operations must support cancellation and timeouts.

### Logging and Observability
- Use `tracing` for runtime logs; avoid ad-hoc `println!` debugging.
- Emit structured events at subsystem boundaries and state transitions.
- Never log secrets (keys, tokens, credentials, private data).

### Security and Tooling Boundaries
- Never persist raw secrets in session or history stores.
- Keep SSH host verification policy explicit and configurable.
- Validate filesystem paths for tool operations.
- Plugin loading must enforce API/manifest compatibility checks.
- SSH interactive flows must support PTY, resize, streaming, and cancellation.

## Documentation and Decision Hygiene
- If you change boundaries, APIs, config schema, persistence, or major dependencies:
  - update `docs/DECISIONS.md`
  - update affected planning/integration docs in `docs/initial-planning/`
- Keep docs in sync with behavior in the same change.

## Clean Code Expectations
- Remove dead code, stale modules, and commented-out blocks.
- Avoid scaffold-only files that provide no immediate behavior.
- Do not leave TODO placeholders for required behavior.

## Definition of Done (Minimum)
- Touched scope compiles.
- Formatting and clippy are clean.
- Relevant tests pass for touched behavior.
- Docs/decisions updated when architecture or config changes.
- Change remains aligned with `big-picture.md` and `integration-plan.md`.
