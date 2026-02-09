# AGENTS.md (Repository Guide for Coding Agents)

This repo is Rustic-AI. Follow this file when making changes.

Current repo note

- Rust workspace is present with core engine and frontend CLI consumer.
- Keep the core UI-agnostic; put interface consumers under `frontend/`.

Read these before changing code

- `docs/DESIGN_GUIDE.md` (non-negotiable workflow + DoD)
- `docs/DECISIONS.md` (ADR-lite decision log)
- `docs/initial-planning/big-picture.md` (north star)
- `docs/initial-planning/integration-plan.md` (boundaries + data flow)
- `docs/initial-planning/REQUIREMENTS.md` (capability + quality baseline)
- `docs/initial-planning/tools.md` (target tool inventory baseline)

No Cursor/Copilot rules were found in `.cursor/rules/`, `.cursorrules`, or
`.github/copilot-instructions.md` at the time this file was written.

Quality Gate (required workflow)

For any non-trivial change, always do:

1. plan: understand current state, decisions, and next step; ask questions if ambiguity matters
2. implement: full implementation (no placeholders/stubs/partials)
3. quality check/validation: run relevant build/lint/test and verify integration
4. fix all issues properly: no deferring, no shortcuts
5. validate against big-picture/integrations: confirm boundaries and goals still hold

Keep the codebase clean

- Avoid bloat and boilerplate-only scaffolding.
- Remove unused/deprecated code and files as part of the same change.
- Do not leave dead modules, commented-out blocks, or TODOs that represent missing behavior.
- When behavior/config/API changes, update docs in the same change.

Build / lint / test commands (Rust workspace)

These assume a Cargo workspace with crates like `rustic-ai-core` and `rustic-ai-cli`.

Build

- Build workspace: `cargo build --workspace`
- Build release: `cargo build --workspace --release`
- Build a crate: `cargo build -p rustic-ai-core`

Format

- Check formatting: `cargo fmt --all -- --check`
- Format code: `cargo fmt --all`

Lint

- Clippy workspace: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Clippy a crate: `cargo clippy -p rustic-ai-core --all-targets --all-features -- -D warnings`

Test

- Test workspace: `cargo test --workspace --all-features`
- Test a crate: `cargo test -p rustic-ai-core --all-features`
- Run a single test (by name substring):
  - `cargo test -p rustic-ai-core my_test_name -- --nocapture`
- Run a single test (exact, with module path):
  - `cargo test -p rustic-ai-core agents::memory::tests::evicts_old_messages -- --exact --nocapture`
- Run tests in one file (usually by module name substring):
  - `cargo test -p rustic-ai-core memory -- --nocapture`

Docs

- Build docs: `cargo doc --workspace --no-deps`

Code style guidelines (Rust)

Formatting

- Use rustfmt defaults; do not hand-format in a conflicting style.
- Keep lines readable; refactor overly long expressions.

Imports

- Group imports in this order, separated by blank lines:
  1) `std::...`
  2) external crates
  3) `crate::...` / `super::...`
- Prefer explicit imports over glob imports.
- Keep `use` lists sorted and minimal.

Naming

- Modules/files: `snake_case`
- Types/traits/enums: `PascalCase`
- Functions/vars: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Prefer descriptive names over abbreviations.

Types and APIs

- Prefer strongly-typed enums/structs over stringly-typed protocols.
- Use newtypes for important IDs/keys when it clarifies meaning.
- Keep public API small; expose constructors/builders instead of public fields.
- Do not leak implementation details across boundaries (e.g., sqlx types outside storage).

Error handling

- No `unwrap()`/`expect()` in core paths.
- Use a typed error enum (`thiserror`) in `rustic-ai-core`.
- Add context at boundaries (provider/tool/storage) with relevant identifiers.
- Prefer actionable error messages (what failed, where, next likely fix).

Async/concurrency

- Use Tokio; avoid blocking operations on async threads.
- Prefer message passing (bounded channels) over shared mutable state.
- When locks are needed, keep lock scope small; avoid holding locks across `.await`.
- All long-running ops must be cancellable (plan for cancellation tokens).
- Enforce timeouts around network and remote execution.

Logging/observability

- Use `tracing` (not ad-hoc prints) inside the engine.
- Emit structured events at subsystem boundaries and state transitions.
- Never log secrets (API keys, tokens, private material).

Security and secrets (pragmatic defaults)

- Do not persist raw secrets in session history/state.
- Keep SSH host key verification policy explicit and configurable.
- Put guardrails around file tools (path validation; consider allow-root configuration).

Plugins and remote execution

- Tool plugins are in-process native code; treat them as trusted unless an out-of-process mode exists.
- Plugin API/manifest must be versioned; reject incompatible plugins with clear errors.
- Interactive SSH requires PTY support: stream stdout/stderr, accept input, handle resize, support cancellation/timeouts.

Docs and decisions

- If you change a boundary, public API, config format, persistence, or a major dependency:
  - update `docs/DECISIONS.md` (new ADR entry or supersede an existing one)
  - update relevant integration/big-picture docs if affected
- Keep documentation relevant; do not add redundant summary docs.

Definition of done (minimum)

- Code compiles for the touched scope.
- Formatting and clippy are clean for the touched scope.
- Tests added/updated when the repo’s testing phase begins (do not add placeholder tests).
- Docs updated for behavior/config/API changes.
- Change aligns with `docs/initial-planning/big-picture.md` and
  `docs/initial-planning/integration-plan.md`.
