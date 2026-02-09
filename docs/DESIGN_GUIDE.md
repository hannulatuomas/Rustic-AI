Design Guide (Principles, Rules, Choices)

Purpose

This document is the consistency anchor for Rustic-AI during development. When a design decision is ambiguous, prefer the rules here. If a change conflicts with this guide, update this guide first (explicit decision) and then implement.

Development Commitment (Non-Negotiable)

- Keep the codebase clean and well structured; follow best practices.
- No bloat, no partial/placeholder implementations, no boilerplate-only stubs.
- Remove unused/deprecated code and files as part of the same change.
- Fix issues properly; changes must compile and work.
- Do not rush; do it right.
- Before implementing, understand what exists, what decisions were made, and what comes next. Ask clarifying questions if ambiguity would change the outcome.
- When code changes, update relevant documentation in the same change.
- Avoid redundant "summary" docs; keep documentation relevant and actionable.
- No skipping, no shortcuts, no deferring, no simplifications.

Required Workflow (Always)

For any non-trivial change, follow this exact loop:

1. Plan
   - Understand current state (code + docs + decisions).
   - Decide approach and boundaries.
   - Ask targeted clarifying questions if needed.
2. Implement
   - Make full, proper implementations (not partial).
   - Keep boundaries and feature flags intact.
3. Quality Check / Validation
   - Build and run relevant checks.
   - Verify integration points (registries, events, storage, config).
4. Fix All Issues Properly
   - If anything fails, fix the root cause (no band-aids).
5. Validate Against Big Picture and Integrations
   - Confirm alignment with:
     - `docs/initial-planning/big-picture.md`
     - `docs/initial-planning/integration-plan.md`
   - Confirm consistency with:
     - `docs/DECISIONS.md`
     - this guide
6. Update TODO (Mandatory)
   - Update `TODO.md` in the same change.
   - Mark completed tasks as done and keep next tasks accurate.
   - Treat `TODO.md` as the single active task tracker.

Definition of Done (Per Change)

- Code compiles; core flows work for the affected area.
- No dead code, unused files, or deprecated leftovers introduced.
- Errors are typed/context-rich; no panics added to core paths.
- Documentation updated where behavior/config/API changed.
- Change fits the integration boundaries (traits/registries/events) and does not leak implementation details across layers.

Project Intent

- Rustic-AI is an agentic AI engine, not a UI.
- The core deliverable is a Rust library that can be embedded by multiple consumers (CLI first; TUI/API later).
- UI consumers are organized under `frontend/` (CLI first; TUI/API/GUI later).
- Feature-rich matters, but not at the cost of a messy architecture.
- Support both direct mode (no project setup) and optional project mode.

Non-Negotiable Principles

Library-First, UI-Agnostic

- `rustic-ai-core` contains all logic: orchestration, storage, providers, tools, workflows, sessions.
- UI consumers (CLI/TUI/API) translate user I/O to engine `Command`s and render engine `Event`s.
- No terminal concepts in the core (no colors, no line editing, no direct stdout/stderr writes).

Loose Coupling via Traits + Registries

- Providers, tools, and storage backends are accessed via traits.
- Agents never depend on concrete provider/tool types.
- Wiring happens at startup: config -> registries -> engine facade.

Bounded Resources

- All queues/channels are bounded.
- All long-running operations are cancellable.
- Context windows are explicit and token-aware.
- Memory growth is bounded (history may be large in storage; working set is bounded).

Correctness and Safety

- No panics in core paths.
- Avoid `unsafe` by default.
- Strong typing over "stringly" protocols.
- Errors carry context (what failed, where, and what was attempted).

Async-First

- All I/O is async (Tokio).
- Avoid blocking calls on async executors.

Core Architecture Rules

Crate Boundaries

- `rustic-ai-core`
  - owns: types, config, providers, tools, storage, sessions, workflows, coordinator, events
  - forbids: clap, terminal UI libraries, printing, reading from stdin
- `frontend/rustic-ai-cli`
  - owns: argument parsing, interactive input, rendering output
  - depends on: `rustic-ai-core`

Stable Engine API

- Public entry point: `RusticAI` facade.
- Inputs: typed `Command`s.
- Outputs: typed `Event` stream + final `Result`.
- Keep public surface small; prefer module-level constructors and strongly-typed options.

Event-Driven Integration

- The engine emits a single unified stream of events.
- Consumers render events differently (CLI text, TUI panes, API SSE/WebSocket).

Minimum Event Set (conceptual)

- `Event::Progress` (workflow/agent/tool milestones)
- `Event::ModelChunk` (streamed LLM output)
- `Event::ToolOutput` (streamed tool stdout/stderr)
- `Event::Error` (recoverable/non-recoverable)
- `Event::SessionUpdated` (durability checkpoints)

Storage and Persistence Rules

Abstraction First

- Everything persists behind `StorageBackend`.
- No SQLite/sqlx types leak outside storage modules.

SQLite First (Default)

- SQLite is the first backend.
- Schema migrations are mandatory.
- Versioning is explicit (schema version stored; migrations are ordered).

Storage vs Memory

- Storage = durable record (sessions/messages/state).
- Memory = working set for prompts and orchestration (bounded context windows).

Configuration Rules

Config Precedence

- Default config file values
- Environment overrides
- Runtime overrides (from consumer)

Configuration Must Be Validated

- Validate at startup:
  - referenced providers/tools exist
  - referenced skills exist
  - required secrets present (or feature disabled)
  - storage connection string valid
- Fail fast with actionable errors.

Rules and Context Scopes

- Support rule/context sources at multiple scopes:
  - global
  - project
  - topic/session
- Apply deterministic precedence:
  - global -> project -> topic/session -> runtime override
- Enforce security-sensitive rules in code, not only prompt text.

Project Mode Rules

- Project mode is optional and must not block direct usage.
- Project profile includes at minimum:
  - project root path
  - tech stack
  - project-scoped rules/context
  - goals and decisions/preferences
  - design/styling guidance references
- Sessions may bind to a project profile; unbound sessions operate in direct mode.

Catalog Taxonomy Rules

- Organize agents/tools/skills with depth-2 hierarchy only:
  - Basket -> Sub-basket
- Items can belong to multiple baskets/sub-baskets.
- Taxonomy is metadata for discovery and filtering, not execution policy.

Feature Toggles

- All non-mandatory subsystems must be toggleable in config/runtime.
- Initial toggle targets include:
  - plugins
  - skills
  - MCP
  - workflow triggers

Dependency and Feature Flag Policy

Keep the Core Lean

- Prefer minimal, widely used crates.
- Use Cargo features to gate optional integrations:
  - provider-specific code
  - plugin loading
  - SSH tool

No Implicit Mega-Features

- Adding a feature must not drag in a large dependency tree unless justified.
- Prefer adding a trait boundary first, then an implementation behind a feature.

Provider Integration Rules

Uniform Message Model

- Normalize to a single internal `ChatMessage` model.
- Providers implement conversion at the boundary.

Streaming and Cancellation

- Streaming must be cancellable.
- Provider calls must enforce timeouts and retry policies.

Graceful Degradation

- Prefer configured fallback chains:
  - provider/model fallback
  - local model fallback (Ollama)
- Degradation should be explicit in events/logs.

Tool Integration Rules

Structured Args

- Tools accept structured JSON args (validated against a schema).
- Tools return `ToolResult` plus streamed output events.

Skills Integration

- Skills are first-class and independent from tools.
- Supported skill formats:
  - instruction-only files (`.md`, `.txt`)
  - script-backed files (`.py`, `.js`, `.ts`)
- Agents can be configured with skills and tools separately.

MCP Integration

- External MCP tools are adapted into the internal `Tool` contract.
- MCP support is behind feature/config toggles.
- MCP failures must follow the same timeout/retry/error model as built-in tools.

Time Limits and Cancellation

- Every tool run has a timeout.
- Tool execution must be cancellable.

Remote Execution (SSH PTY)

- Interactive SSH is a first-class requirement.
- The SSH tool must support:
  - PTY allocation
  - stdout/stderr streaming
  - input forwarding (bytes/lines)
  - terminal resize events
  - host key policy and timeouts

Permission System Rules

- Sensitive operations require policy decisions.
- Decision model:
  - allow
  - deny
  - ask
- Ask outcomes:
  - allow once
  - allow in session
  - deny
- Persist decisions by scope (session/project/global) with audit metadata.

Workflow Trigger Rules

- Workflow model must support triggers (manual, schedule, webhook/event) as metadata.
- Runtime remains headless; future visual editors consume the same graph/trigger model.

Requirements Inputs

- Keep implementation aligned with:
  - `docs/initial-planning/REQUIREMENTS.md`
  - `docs/initial-planning/tools.md`
- If scope changes, update ADRs and planning docs in the same change.

Plugin System Rules (Tools First)

Compatibility and Versioning

- Plugin API is versioned.
- Incompatible plugins must be rejected with clear diagnostics.

Trust Model

- Plugins execute native code in-process.
- Treat plugins as trusted code unless a sandbox is explicitly implemented.
- Document this clearly; do not imply safety that is not present.

Coding Rules and Preferences

Error Handling

- Core uses a typed error enum (`thiserror`).
- Add context at boundaries (provider/tool/storage) with relevant identifiers.

Types and Enums

- Prefer enums/structs over strings.
- Prefer newtypes for IDs and important values (when it improves clarity).

Concurrency

- Prefer message passing (channels) for cross-task coordination.
- Minimize write-lock contention; keep lock scopes small.

Logging/Observability

- Use `tracing`.
- Log at boundaries and state transitions (start/end of provider calls, tool runs, workflow steps).
- Never log secrets.

Security Defaults (Pragmatic)

- Never persist raw secrets in session history.
- Keep SSH host key verification policy explicit (configurable).
- Put guardrails around file tools (path validation, explicit allow roots when appropriate).

Decision Log (Current)

- Engine is library-first; CLI is first consumer.
- Persistence uses a storage trait; SQLite is first implementation.
- Tools support dynamic plugins early (tools first).
- SSH remote execution must support full interactive PTY.
- Async runtime: Tokio.
- Logging: tracing.

How To Propose Changes

When a new requirement conflicts with this guide:

1. Update this file with the new rule/choice and rationale.
2. Update integration/big-picture docs if they are impacted.
3. Implement the change with minimal churn outside the boundary.
