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

ADR-0011: Rule and Context Discovery Runtime Model

- Status: Accepted
- Date: 2026-02-09
- Context: Rules/context files need deterministic precedence across global, project, and topic/session scopes, with project-specific overrides based on where the engine is executed.
- Decision:
  - Resolve discovery roots from the runtime working directory (where the CLI is launched), not from the Rustic-AI repository root.
  - Add configurable discovery controls in config:
    - global rules base path (default `~/.rustic-ai/rules`)
    - project rules folder (default `.agents`)
    - additional search paths
    - recursive toggle, max depth (default 5), and `.gitignore` filtering toggle
    - configurable rule extensions/file names and configurable context patterns/extensions
    - configurable topic debouncing (`30s`) and similarity threshold (`0.5`)
  - Parse rule frontmatter as JSON metadata with applicability (`general` or `context_specific`), topics, scope hint, and priority.
  - Store discovered rule metadata as paths + descriptors in config; rule content loading remains deferred to session/agent layers.
  - Sort discovered rules with deterministic precedence (`global -> project -> topic/session`) and then by proximity to the runtime working directory.
- Consequences:
  - Foundation now supports rule/context auto-discovery with override behavior aligned to the user’s active project folder.
  - Session/agent layers can filter context-specific rules using discovered metadata without re-scanning filesystem each turn.
  - Topic inference and rule materialization are now explicit follow-up integration tasks.

---

ADR-0012: JSON Config Format and Storage Path Strategy

- Status: Accepted
- Date: 2026-02-09
- Context: We need consistent, user-friendly configuration and predictable storage locations across project-scoped sessions and global preferences.
- Decision:
  - Use JSON as the default runtime config format (`config.json`) for loader and CLI defaults.
  - Keep global reusable data under `~/.rustic-ai/` with explicit separation:
    - `~/.rustic-ai/settings.json` for global settings
    - `~/.rustic-ai/data/` for global data/cache
  - Keep project/session data under runtime working directory defaults:
    - `<workdir>/.rustic-ai/sessions.db`
  - Keep all path behavior configurable from config.
  - Extend rule frontmatter metadata to support Cursor-like UX semantics (`description`, `globs`, `alwaysApply`) while preserving Rustic-AI scope/topic metadata and manual `@rule-name` invocation.
- Consequences:
  - Config and CLI setup are consistent around JSON by default.
  - Session persistence aligns to active project working directory, while global settings remain reusable across projects.
  - Rule selection now supports always-apply, file-pattern activation, context-specific topics, and manual invocation with deterministic precedence.

---

ADR-0013: Storage Backend Factory and Explicit Backend Selection

- Status: Accepted
- Date: 2026-02-10
- Context: Runtime/storage wiring previously referenced SQLite directly in core initialization, making backend replacement harder.
- Decision:
  - Add explicit storage backend selection in config via `storage.backend` (`sqlite`, `postgres`, `custom`).
  - Introduce backend-specific config namespaces under `storage`:
    - `storage.sqlite.*` for SQLite runtime settings
    - `storage.postgres.*` reserved for future PostgreSQL implementation
  - Introduce storage factory boundary (`storage::create_storage_backend`) and remove direct SQLite construction from `RusticAI::new`.
  - Keep `StorageBackend` trait as the only persistence boundary exposed to conversation/runtime layers.
  - For unimplemented backends, fail fast with explicit errors:
    - `storage backend 'postgres' is not implemented yet`
    - `storage backend 'custom' is not implemented yet`
- Consequences:
  - Storage backend can be swapped with minimal impact to runtime/session code.
  - Backend-specific tuning is configurable and isolated.
  - Unknown/unimplemented backend selection fails clearly at startup.

---

ADR-0014: Provider Factory and Explicit Provider Wiring

- Status: Accepted
- Date: 2026-02-10
- Context: Runtime had provider-specific wiring logic directly in `Runtime::new`, making provider extension and replacement harder.
- Decision:
  - Introduce provider factory boundary (`providers::create_provider_registry`) and move provider-specific construction out of runtime.
  - Make `Runtime::new` return `Result<Self>` and fail fast when provider setup cannot be completed.
  - Require explicit provider config and explicit env var availability; do not use provider-specific fallback values.
  - Return clear "not implemented yet" errors for configured provider types that are not yet available.
- Consequences:
  - Runtime is provider-agnostic and easier to extend for additional provider implementations.
  - Misconfiguration errors are surfaced during startup with actionable error messages.
  - Provider onboarding follows a single factory extension point.

---

ADR-0015: Provider Config Schema Is Type-Specific and Extensible

- Status: Accepted
- Date: 2026-02-10
- Context: A single provider schema requiring all fields for all provider types makes future provider onboarding brittle and forces runtime reshaping.
- Decision:
  - Use type-specific schema requirements for providers:
    - `open_ai` requires explicit `model`, `api_key_env`, and `base_url`.
    - Other provider types currently require generic identity/auth fields and may carry provider-specific `settings`.
  - Add optional `settings` object to provider config for forward-compatible, provider-specific options.
  - Keep strict CLI validation aligned with type-specific requirements.
- Consequences:
  - New providers can be added incrementally without breaking existing config shape.
  - Runtime/provider factory remains stable while provider-specific options evolve.
  - Validation remains explicit and deterministic for currently implemented providers.

---

ADR-0016: Config Mutation Layer with Typed Paths and Atomic Persistence

- Status: Accepted
- Date: 2026-02-10
- Context: Multiple frontends (CLI/TUI/GUI/REST) need consistent config reads/partial writes without rewriting whole files manually.
- Decision:
  - Add `ConfigManager` in core as the config mutation boundary.
  - Add typed `ConfigPath` and `ConfigScope` for stable, non-stringly config access/update APIs.
  - Support partial updates via patch list (`ConfigChange`) with all-or-nothing apply semantics.
  - Support effective config reads with source attribution (project/global/default).
  - Allow explicit scope-targeted writes (`project` or `global`) so local overrides can differ from effective source.
  - Persist updates atomically using temp-file write + fsync + rename.
  - Validate effective config after patches before commit/persist.
  - Keep session-scope writes explicitly unimplemented for now with clear errors.
  - Add stable machine-readable CLI output envelope for config commands:
    - schema id: `rustic-ai-cli/config-output/v1`
    - success fields: `schema`, `status`, `command`, `data`
    - failure fields: `schema`, `status`, `command`, `error{code,message,details}`
- Consequences:
  - Frontends can implement get/set/patch behavior consistently without custom file mutation logic.
  - Config writes are safer against partial/corrupt file states.
  - Frontend adapters (TUI/GUI/REST bridge) can depend on a stable JSON output contract.
- Future hot-reload and event propagation can integrate with `ConfigManager` as a single change source.

---

ADR-0017: Subscription Authentication for OpenAI (Browser + Headless)

- Status: Accepted
- Date: 2026-02-10
- Context: Users need to run OpenAI provider using subscription-style authentication (Plus/Pro-compatible token flow) instead of only static API keys, including browser and headless login UX.
- Decision:
  - Add a core auth subsystem with:
    - OAuth browser flow (authorization code + PKCE + local callback listener)
    - OAuth device flow (headless / command-line authentication)
    - Local credential store under global data path (default `~/.rustic-ai/data/auth.json`)
    - Runtime token refresh using refresh tokens when available
  - Add CLI auth commands:
    - `auth connect --provider <name> --method browser|headless`
    - `auth list`
    - `auth logout --provider <name>`
  - Allow OpenAI `auth_mode: subscription` without requiring `api_key_env`.
  - Keep OpenAI request headers auth-mode aware and inject subscription bearer tokens dynamically per request.
- Consequences:
  - Subscription-mode OpenAI operation no longer depends on manual API-key env vars.
  - Credential lifecycle (acquire, store, refresh, revoke local copy) is handled by Rustic-AI instead of external manual token handling.
- Other providers can adopt the same auth subsystem incrementally.

---

ADR-0018: Provider-Scoped Auth Mode Capabilities

- Status: Accepted
- Date: 2026-02-10
- Context: Not all providers support all auth modes (for example, Grok currently supports API key auth only). We need explicit, user-visible enforcement to prevent invalid config and confusing auth flows.
- Decision:
  - Add central provider auth capability mapping in core (`providers/auth_capabilities.rs`).
  - Validate configured `auth_mode` against provider capabilities and return errors listing supported auth modes.
  - Add CLI command `auth methods` to show configured providers, configured auth mode, and supported auth modes.
  - Block `auth connect` when provider/auth-mode configuration does not support subscription authentication.
- Consequences:
  - Invalid provider/auth combinations fail early with actionable guidance.
  - Users can discover supported auth methods without reading source code.
- Subscription auth can be rolled out provider-by-provider without breaking existing providers.

---

ADR-0019: Expanded Provider Matrix (Grok, Z.ai, Custom OpenAI-Compatible, Ollama)

- Status: Accepted
- Date: 2026-02-10
- Context: We need broader provider coverage while keeping authentication support explicit and predictable.
- Decision:
  - Implement Grok provider with API-key auth only.
  - Implement Z.ai provider with two endpoint families (general/coding) and explicit endpoint profile selection.
  - Implement Custom provider as OpenAI-compatible (`/chat/completions`) with endpoint + API key.
  - Implement Ollama provider with chat, stream, and token-count endpoints, authenticated via API key.
  - Keep subscription authentication available only for OpenAI, Anthropic, and Google.
- Consequences:
  - Provider behavior is explicit and discoverable via `auth methods`.
  - Config validation fails fast for unsupported auth/provider combinations.
  - Additional providers are usable without introducing fake/untested subscription paths.

---

ADR-0020: Layered Configuration via Global/Project Fragment Files

- Status: Accepted
- Date: 2026-02-10
- Context: A single monolithic config file is hard to maintain as provider/tool/agent settings grow. We need easier separation similar to OpenCode-style organization while preserving deterministic merge behavior.
- Decision:
  - Keep `config.json` as the canonical base config.
  - Add automatic JSON fragment loading from:
    - global: `~/.rustic-ai/config/*.json` (or `<storage.global_root_path>/config/*.json` when configured)
    - project: `<workdir>/<storage.default_root_dir_name>/config/*.json` (typically `.rustic-ai/config/*.json`)
  - Merge order is deterministic: base `config.json` -> global fragments (sorted by filename) -> project fragments (sorted by filename) -> env overrides.
  - Fragments are partial JSON objects and can override only the sections they define (agents/tools/providers/permissions/etc).
- Consequences:
  - Users can split config concerns into separate files without losing a single effective config model.
  - Project config can override global defaults cleanly.
- Env vars remain highest-precedence runtime override path.

---

ADR-0021: Phase 6 Learning Subsystem (Feedback, Patterns, Preferences, Success Library)

- Status: Accepted
- Date: 2026-02-11
- Context: The roadmap requires a self-learning loop so the system can reduce repeated mistakes, adapt to user preferences, and capture successful task patterns.
- Decision:
  - Add a dedicated `learning` module in `rustic-ai-core` with typed models and manager APIs for:
    - explicit and implicit feedback collection
    - mistake pattern classification/tracking
    - user preference recording/retrieval
    - success pattern extraction/similarity/reuse
  - Extend `StorageBackend` with learning persistence operations and add schema version 3 migrations for SQLite/Postgres.
  - Integrate learning with agent turns:
    - pre-turn pattern warnings
    - post-turn success pattern recording
    - error/mistake recording
    - optional preference application to prompt context
  - Add interactive CLI feedback capture via `/feedback --type <explicit|success|error> --rating <-1..1> [--comment <text>]`.
  - Add `features.learning_enabled` to allow runtime enable/disable.
- Consequences:
  - Learning data is now durable and backend-agnostic through the storage trait boundary.
  - Agent behavior can adapt gradually based on persisted user/task history.
  - Event stream includes learning lifecycle notifications for UI consumers.

---

ADR-0022: Phase 7 Code Indexing Foundation with Persistent Symbol Store

- Status: Accepted
- Date: 2026-02-11
- Context: Phase 7 requires big-codebase support. We need an immediately usable indexing baseline while tree-sitter integration is still pending.
- Decision:
  - Add a new `indexing` module in core with:
    - `CodeIndex`, `FileIndex`, `SymbolIndex`, and `SymbolType` typed models
    - tree-sitter-backed parser/symbol extraction for Rust, Python, JS/TS, Go, and C/C++
    - call graph extraction/persistence from parsed call expressions
    - indexer APIs for full rebuild, per-file incremental updates, and symbol search
  - Extend `StorageBackend` with persistent code index operations and add schema version 4 migrations for SQLite/Postgres:
    - `code_index_metadata`
    - `code_file_indexes`
    - `code_symbol_indexes`
    - `code_call_edges`
  - Use deterministic persisted snapshots as the retrieval backbone for upcoming vector/RAG phases.
- Consequences:
  - Workspace indexing is now durable across restarts and backend-agnostic.
  - Symbol retrieval and call graph discovery are available before vector search/RAG integration.
  - Tree-sitter grammars are now an explicit dependency boundary for indexing fidelity.

---

ADR-0023: Phase 7 Vector Store Baseline (Persistent + Cosine Search)

- Status: Accepted
- Date: 2026-02-11
- Context: RAG integration requires vector persistence and retrieval before provider-specific embedding integrations are finalized.
- Decision:
  - Add a `vector` module with typed `Embedding`, `SearchQuery`, `SearchResult`, and `VectorDb` APIs.
  - Introduce a pluggable `EmbeddingProvider` trait and ship a deterministic local baseline provider for development workflows.
  - Extend `StorageBackend` with vector persistence operations and add schema version 5 migrations for SQLite/Postgres (`vector_embeddings`).
  - Implement cosine similarity ranking in core as the default retrieval strategy.
- Consequences:
  - Vector retrieval is available now for upcoming hybrid keyword+semantic RAG flows.
  - External embedding providers (OpenAI/local/SBERT) can be added without changing vector persistence contracts.

---

ADR-0024: Configurable Hybrid RAG Context Injection in Agent Turns

- Status: Accepted
- Date: 2026-02-11
- Context: Big codebase support requires retrieval-augmented prompting while keeping operational control over performance and behavior.
- Decision:
  - Add a `rag` module with hybrid retrieval (keyword symbol lookup + vector similarity), typed retrieval request/response models, and prompt formatting.
  - Inject retrieval context into agent turns before model generation, with event emission for observability.
  - Make indexing/vector/RAG behavior configurable and independently toggleable via:
    - `features.indexing_enabled`
    - `features.vector_enabled`
    - `features.rag_enabled`
    - `retrieval.*` tunables (top-k, snippet limits, vector dimension, min score, injection mode)
  - Keep retrieval injection safe by honoring disable switches and returning empty retrieval output when disabled.
- Consequences:
  - Operators can tune or disable heavy retrieval features per environment.
  - Agent context quality improves for large workspaces without hard-coding retrieval behavior.

---

ADR-0025: Configurable Embedding Backends and RAG-Aware Context Compaction

- Status: Accepted
- Date: 2026-02-11
- Context: Retrieval quality depends on embedding backend selection per environment, and large retrieval payloads can crowd out conversation history.
- Decision:
  - Extend retrieval config with explicit embedding backend settings:
    - `embedding_backend`: `deterministic_hash`, `open_ai`, `open_ai_compatible`, `sentence_transformers`
    - `embedding_model`, `embedding_base_url`, `embedding_api_key_env`
  - Implement embedding providers for OpenAI/OpenAI-compatible `/embeddings` and sentence-transformers local `/embed` JSON API.
  - Use the selected embedding provider for both index-time vector generation and query-time retrieval embedding.
  - Add RAG-aware context compaction in the agent turn path to keep token usage within configured context window after retrieval injection.
- Consequences:
  - Deployments can switch embedding strategy without code changes.
  - Retrieval and indexing stay dimension-consistent across backend choices.
  - Large retrieval blocks no longer force unbounded context growth.

---

ADR-0026: Optional SQLite Vector Extension Loading with Strict/Best-Effort Modes

- Status: Accepted
- Date: 2026-02-11
- Context: Some deployments want SQLite-native vector acceleration while others run without native extensions.
- Decision:
  - Add configurable SQLite vector extension controls:
    - `storage.sqlite.vector_extension_enabled`
    - `storage.sqlite.vector_extension_path`
    - `storage.sqlite.vector_extension_entrypoint`
    - `storage.sqlite.vector_extension_strict`
  - Attempt loading extension during SQLite runtime initialization via `load_extension(...)`.
  - Respect strictness:
    - strict mode: initialization fails if extension cannot be loaded
    - best-effort mode: extension load failures are tolerated and core cosine search remains available
- Consequences:
  - Vector acceleration can be enabled per environment without hard dependency on extension availability.
  - Existing behavior remains stable when extension loading is disabled.

---

ADR-0027: Code Graph and Impact Analysis via Index Snapshot

- Status: Accepted
- Date: 2026-02-11
- Context: With indexing and call-edge persistence in place, engineering workflows need direct impact analysis and graph visualization outputs.
- Decision:
  - Add graph analysis utilities on top of index snapshots:
    - build graph nodes/edges from symbols, call edges, and dependencies
    - reverse-call impact traversal from a root symbol with depth limit
    - DOT rendering for graph/impact visualization
  - Expose CLI diagnostics under `index` command surface:
    - `index graph` with `summary|json|dot`
    - `index impact <symbol>` with `summary|json|dot`
    - `index retrieve` filters (`path_prefix`, `kind`) for retrieval diagnostics
- Consequences:
  - Developers can inspect structural impact before edits.
  - Graph/impact outputs are available without external tooling dependencies.

---

Template (copy/paste)

ADR-XXXX: <Title>

- Status: Proposed
- Date: YYYY-MM-DD
- Context:
- Decision:
- Consequences:
