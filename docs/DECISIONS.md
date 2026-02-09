Decisions (ADR-Lite)

This file records project decisions that affect architecture, public APIs, dependencies, or long-term maintainability.

Rules

- If a change would contradict a recorded decision, add a new ADR entry that supersedes it.
- Prefer small, explicit entries over implicit drift.
- Keep entries factual: context -> decision -> consequences.

Format

- ID: ADR-XXXX (increment)
- Status: Proposed | Accepted | Superseded
- Date: YYYY-MM-DD

---

ADR-0001: Project Identity and Scope

- Status: Accepted
- Date: 2026-02-09
- Context: We want a "perfect" agentic system but do not want to lock to a UI early.
- Decision: The project is named Rustic-AI. The primary deliverable is a Rust engine/library (`rustic-ai-core`) with thin consumer UIs (CLI first).
- Consequences:
  - Core must remain UI-agnostic (no terminal I/O, no clap, no rendering).
  - CLI/TUI/API are adapters that translate user input into engine commands and render engine events.

---

ADR-0002: Storage Abstraction and SQLite-First Persistence

- Status: Accepted
- Date: 2026-02-09
- Context: Long-running sessions require durable history/state, and we want to be able to swap storage backends later.
- Decision: Introduce a `StorageBackend` trait in `rustic-ai-core`. SQLite (via `sqlx`) is the first implementation.
- Consequences:
  - No SQLite/sqlx types leak outside the storage module.
  - Migrations and schema versioning are mandatory.
  - Additional backends (Postgres, in-memory) can be added later without changing orchestration logic.

---

ADR-0003: Interactive SSH Remote Execution (PTY)

- Status: Accepted
- Date: 2026-02-09
- Context: Remote execution is a core feature and must support full interactive sessions.
- Decision: The SSH tool must support interactive PTY sessions (input forwarding, output streaming, resize events, cancellation, timeouts, host key policy). Use a Rust SSH library that supports PTY (planned: `russh`).
- Consequences:
  - Engine exposes an interactive tool-session handle that is UI-agnostic.
  - Consumers (CLI/TUI) decide how to capture keystrokes; the engine consumes bytes/lines.

---

ADR-0004: Tool Plugins (In-Process, Tools First)

- Status: Accepted
- Date: 2026-02-09
- Context: We want extensibility without recompiling the engine.
- Decision: Implement a tool plugin system early. Plugins are dynamically loaded in-process (planned: `libloading`).
- Consequences:
  - Plugins are trusted native code unless/until an out-of-process sandbox mode is built.
  - Plugin API/manifest must be versioned; incompatible plugins are rejected with clear errors.
  - Plugin loading is behind a Cargo feature to avoid dependency bloat when not used.

---

ADR-0005: Observability and Error Handling

- Status: Accepted
- Date: 2026-02-09
- Context: The engine is async and multi-component; we need consistent diagnostics.
- Decision: Use `tracing` for structured logs/events. Use typed errors (`thiserror`) with context at subsystem boundaries.
- Consequences:
  - No `unwrap`/`expect` in core paths.
  - Never log secrets.
  - Provider/tool/storage boundaries attach identifiers and actionable context to errors.

---

ADR-0006: Dependency and Feature-Flag Policy

- Status: Accepted
- Date: 2026-02-09
- Context: The project must stay clean and avoid bloat while still being feature-rich.
- Decision: Keep `rustic-ai-core` lean by gating optional providers/tools/plugins behind Cargo features. Add dependencies only with clear justification.
- Consequences:
  - Provider integrations are opt-in features.
  - Tools that require heavy deps are opt-in features.
  - The default feature set remains practical for a CLI-first setup.

---

ADR-0007: Development Workflow and Quality Gate

- Status: Accepted
- Date: 2026-02-09
- Context: The project must be clean, correct, and consistent. We explicitly do not want shortcuts, partial implementations, or documentation drift.
- Decision: For all non-trivial work, follow: plan -> implement -> quality check/validation -> fix all issues properly -> validate against big-picture/integration docs -> update TODO. Documentation updates are part of the same change when behavior/config/API changes.
- Consequences:
  - No partial/placeholder implementations; features are implemented fully and integrated.
  - Build must pass for the touched scope; failures are fixed (not deferred).
  - Unused/deprecated code/files are removed as part of the change.
  - Planning requires understanding the current repo state and prior decisions; ask clarifying questions when ambiguity materially changes the outcome.
  - Documentation remains relevant; avoid creating redundant summary docs.
  - `TODO.md` is the single active tracker and is updated in every non-trivial change.

---

ADR-0008: Frontend Layout, Policy Model, and Workflow Capabilities

- Status: Accepted
- Date: 2026-02-09
- Context: We need a clean project structure for multiple UI options and clear capability requirements before implementation deepens.
- Decision:
  - UI consumers live under `frontend/` (starting with `frontend/rustic-ai-cli`).
  - Rules/context support must include global, project, and topic/session scopes with deterministic precedence.
  - Non-mandatory subsystems must be toggleable (plugins, skills, MCP, triggers).
  - Skills are first-class (instruction and script-backed), and MCP tools are supported through adapters.
  - Permission system must enforce allow/deny/ask with ask outcomes (allow once, allow in session, deny).
  - Workflow model must support trigger metadata for future n8n-style visual builders.
- Consequences:
  - Core APIs and config schemas must model scopes, toggles, permissions, skills, and triggers early.
  - UI layers can evolve independently while reusing the same engine contracts.
  - Tool execution paths must integrate permission checks and audit records.

---

ADR-0009: Project Profiles and Basket Taxonomy

- Status: Accepted
- Date: 2026-02-09
- Context: We need both quick-start usage and richer project-scoped guidance, plus scalable organization for a large agent/tool catalog.
- Decision:
  - Support two modes:
    - direct mode (no project setup; work in current/root directory)
    - project mode (optional profile with root, stack, scoped rules/context, goals, decisions/preferences, style guidance)
  - Add taxonomy model with depth limit 2:
    - Basket -> Sub-basket
  - Allow many-to-many membership for agents/tools/skills.
- Consequences:
  - Session model must support optional project binding.
  - Config and storage must represent project profiles and scoped overlays.
  - Catalog APIs/UI must support taxonomy queries and filtering.
  - Execution logic must not depend on taxonomy metadata.

---

ADR-0010: Requirements and Tool Inventory as Planning Inputs

- Status: Accepted
- Date: 2026-02-09
- Context: Scope is broad and evolving; we need stable source documents to avoid drift.
- Decision:
  - Treat `docs/initial-planning/REQUIREMENTS.md` as capability/quality baseline.
  - Treat `docs/initial-planning/tools.md` as target tool surface baseline.
  - Track major coverage gaps in implementation plans and ADRs.
- Consequences:
  - Planning updates must reference these files when scope changes.
  - Roadmap prioritization should be justified against these baselines.

---

Template (copy/paste)

ADR-XXXX: <Title>

- Status: Proposed
- Date: YYYY-MM-DD
- Context:
- Decision:
- Consequences:
