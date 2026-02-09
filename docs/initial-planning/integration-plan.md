Integrations Plan

This document explains how the major subsystems fit together without becoming a tangled monolith. It is not a task list; it is an interface and data-flow guide that we will keep aligned with the implementation as the codebase grows.

Integration North Star

- Library-first engine: `rustic-ai-core` owns the runtime, state, and orchestration.
- Thin consumers (CLI/TUI/API/GUI) adapt I/O and render events; they do not contain business logic.
- Consumer crates live under `frontend/` (first: `frontend/rustic-ai-cli`).
- Loose coupling: traits + registries + typed events; minimal cross-module knowledge.
- Bounded resources: context windows, queues, timeouts, cancellation.

High-Level Layering

1. UI Consumer Layer (optional, replaceable)
   - CLI today; TUI/API/GUI later
   - Subscribes to events (streaming tokens, progress, tool output)
   - Sends commands (user input, slash commands, workflow invocations)

2. Engine Facade (`RusticAI`)
   - Initializes from config
   - Owns registries and managers
   - Provides stable public API

3. Orchestration Layer
   - Session/Conversation Manager (history + context windows)
   - Coordinator (workflows, parallelism, agent scheduling)
   - Project Manager (optional project profiles and scoped settings)

4. Execution Layer
   - Providers: model calls + streaming
   - Tools: local/remote execution + streaming
   - Skills: reusable instruction/script units bound to agents/workflows
   - MCP adapter: external tool discovery/invocation through MCP
   - Plugins: dynamic tool loading and registration

5. Persistence Layer
   - Storage trait
   - SQLite backend (migrations, schema versioning)

Key Interfaces (Traits)

- `StorageBackend`: sessions, messages, state; swappable backend
- `ModelProvider`: generate + stream_generate; uniform message model
- `Tool`: execute + stream outputs; input is structured JSON args
- `Skill`: typed skill contract for instruction and script-backed skills
- `PermissionPolicy`: allow/deny/ask enforcement with scope-aware decisions

Core Data Types (Stable)

- `ChatMessage` (role + content + metadata)
- `Session` (id + agent binding + timestamps + metadata)
- `ProjectProfile` (root path, stack, scoped rules, goals, decisions/preferences, style guidance)
- `Workflow` / `Step` (typed orchestration)
- `Trigger` (event/source that starts workflows)
- `Event` (engine -> consumer stream)
- `Command` (consumer -> engine input)
- `Basket` / `SubBasket` metadata (depth 2) for agent/tool/skill organization

End-to-End Data Flow

```
UI Consumer (CLI/TUI/API)
  |  Command::UserInput / Command::Slash / Command::RunWorkflow
  v
Engine Facade (RusticAI)
  |  loads Config
  |  builds registries + managers
  v
Project Manager --> Session Manager <-----------+
  | creates/loads Session                        |
  | persists history/state via StorageBackend     |
  v                                               |
Coordinator (Workflows + Agent Scheduling)       |
  | spawns tasks (tokio)                          |
  | emits Event::Progress                         |
  v                                               |
Agent (act loop)                                 |
  | builds prompt/context via Memory + Rules      |
  | calls ProviderRegistry                        |
  | requests Tool execution via ToolRegistry      |
  v                                               |
Provider / Tool Execution                         |
  | streams tokens/output as Event::StreamChunk   |
  | returns final outputs                          |
  v                                               |
Session Manager (append messages, update state) -+
  |
  v
UI Consumer renders events to user
```

Registries and Managers

Registry Layer

- `ProviderRegistry`: name -> `Arc<dyn ModelProvider>`
- `ToolRegistry`: name -> `Arc<dyn Tool>`
- `AgentRegistry`: name -> `Arc<Agent>` (constructed from provider/tool references)

Manager Layer

- `SessionManager`: create/resume sessions, append history, apply context windowing
- `ToolManager`: execute tools with timeouts, streaming, cancellation
- `WorkflowExecutor`: interpret `Workflow` steps into agent/tool operations

Config -> Runtime Wiring

One-time wiring at startup (with an optional future hot-reload hook):

1. Load config from file + env overrides
2. Initialize logging/tracing
3. Initialize storage backend (SQLite by default)
4. Build ProviderRegistry from provider configs
5. Build ToolRegistry from tool configs
6. Build SkillRegistry from skill configs and files
7. Load tool plugins and register plugin-provided tools
8. Initialize MCP adapters and register MCP tools (if enabled)
9. Build AgentRegistry from agent configs (validate provider/tool/skill references)
10. Construct `RusticAI` facade with registries/managers

Project Mode Integration

- If project mode is enabled:
  - Resolve project root and tech stack metadata.
  - Load project-scoped rules, decisions/preferences, goals, and style guidance.
  - Merge with global defaults and topic/session overrides.
- If project mode is disabled:
  - Operate directly from current working directory.
  - Keep global + topic/session rules active.

Rules and Context Files Integration

Inputs

- Rules files: `.cursorrules`, `.windsurfrules`, and similar
- Context files: `AGENTS.md`, `CLAUDE.md`, project-specific docs
- Topic-scoped rule/context files attached at session/workflow level
- Project profile rule/context sources (optional)

Integration Pattern

- Parse into structured "policy" + "context" artifacts:
  - Policy: constraints and preferences (what must/must not happen)
  - Context: reference material (what the system should know)
- Resolve with explicit precedence:
  - global -> project -> topic/session -> runtime override
- Attach to:
  - session-scoped system prompt layer (applies to a session)
  - agent-scoped system prompt layer (applies to an agent)
  - tool allow/deny filters (enforced by coordinator/tool manager)

Important: rules must be enforceable outside the model prompt where possible (e.g., tool allowlists enforced by code).

Permission System Integration

- Central `PermissionPolicy` evaluates sensitive actions (tool calls, remote ops, file writes, network actions).
- Decision modes:
  - allow
  - deny
  - ask
- Ask can resolve to:
  - allow once
  - allow for session
  - deny
- Permission decisions emit events and are persisted with scope (session/project/profile) for auditability.

Model Providers Integration

Uniform Provider Interface

- Agents operate on `Vec<ChatMessage>` and `GenerateOptions`.
- Providers hide API details (OpenAI/Anthropic/Grok/Google/Ollama).

Streaming

- Provider streams tokens/chunks through a channel.
- Engine converts them into `Event::ModelChunk { session_id, agent, text }`.

Resilience

- Retry strategy sits around provider calls (rate limits, timeouts).
- Fallback strategy can be configured per agent or per request:
  - primary provider/model
  - secondary provider/model
  - local provider fallback (Ollama)

Tools Integration (Local + Remote)

Tool Execution Contract

- Input is structured JSON args (validated by the tool)
- Output is:
  - streaming: stdout/stderr chunks -> `Event::ToolOutput`
  - final: `ToolResult` (success + output + metadata)

Interactive SSH PTY (Full Interactive)

- The SSH tool must support a PTY session for interactive commands.
- Integration requirements:
  - allocate PTY
  - stream remote stdout/stderr as events
  - accept input events from UI (keystrokes/lines) when in interactive mode
  - handle terminal resize events
  - enforce timeouts and cancellation

To keep UI-agnostic behavior:

- Engine exposes an interactive "tool session" handle (send input, receive output events).
- CLI/TUI decides how to capture keystrokes; engine just consumes bytes/lines.

Plugins Integration (Tools First)

Goal

- Allow drop-in tools without recompiling the engine.

Integration Contract

- Plugin discovery from config (paths) or a plugins directory
- Plugin loads tools + metadata (name, description, JSON schema)
- ToolRegistry registers them like any built-in tool

Skills Integration

- Skills are attachable to agents and workflows independently of tools.
- Supported formats:
  - instruction skills (`.md`, `.txt`)
  - script-backed skills (`.py`, `.js`, `.ts`)
  - packaged skill bundles
- Skills are validated, versioned, and loaded through a dedicated registry.

MCP Integration

- MCP adapters expose external tools through the same `Tool` contract used by built-in tools.
- Discovery and capability metadata are mapped into tool registry entries.
- MCP feature is toggleable and isolated behind feature/config flags.

Basket / Sub-Basket Integration

- Taxonomy model:
  - Basket (e.g., Programming)
  - Sub-basket (e.g., Rust, Python, C++)
- Constraints:
  - Max depth is 2 (Basket -> Sub-basket).
  - Items may belong to multiple baskets/sub-baskets.
- Purpose:
  - Discovery/filtering/routing metadata for UI/API and catalog views.
  - Not a replacement for permission policies or execution constraints.

Compatibility

- Define a versioned plugin API/manifest
- Engine refuses incompatible plugins with a clear error
- Prefer feature flags to avoid pulling plugin dependencies when not used

Storage Integration (SQLite First, Swappable)

Storage Trait Boundary

- Everything persists through `StorageBackend`.
- No SQLite types leak above the storage module.

SQLite Implementation Details

- Migrations are mandatory; schema version stored in DB
- Sessions/messages/state tables are indexed for common queries
- Writes are batched where possible; history is bounded

Session + Memory + Context Windowing

Separation of Concerns

- Storage = durability (what happened)
- Memory = working set (what to send to the model)

Integration

- SessionManager reads history from storage on resume.
- Memory builds a context window from history:
  - keep recent turns
  - retain pinned/important messages
  - summarize older content (provider-assisted)
- Only the resulting context window goes to the provider.

Workflows / Slash Commands Integration

- Slash commands compile to `Workflow` (typed steps).
- Slash commands can route to workflows, saved prompts, or built-in operational commands.
- WorkflowExecutor interprets steps:
  - Agent step: invoke agent act loop with input
  - Tool step: execute tool via ToolManager
  - Parallel step: spawn tokio tasks, aggregate results
  - Conditional/Loop steps: evaluate based on workflow variables

Workflow Triggers and UI-Ready Graph Model

- Workflow model supports triggers (manual, schedule, webhook/event) as first-class metadata.
- Runtime remains headless; future visual workflow editors (n8n-style) consume the same graph model via API/UI layer.

Progress and Status Integration

Single Event Stream

- The engine emits a unified stream of typed events:
  - `Event::Progress` (workflow/agent/tool milestones)
  - `Event::ModelChunk` (streaming model output)
  - `Event::ToolOutput` (streaming tool output)
  - `Event::Error` (non-fatal and fatal)
  - `Event::SessionUpdated` (durability checkpoints)

Consumers can render these differently (plain text for CLI, panels for TUI, SSE/WebSocket for API).

Error Handling and Graceful Degradation

Where errors live

- Providers/tools/storage return typed errors with context.
- Coordinator decides whether to:
  - retry
  - fall back to another provider/tool
  - degrade features (disable streaming, shorten context, switch model)
  - stop the workflow

Always record

- Persist error events to session history/state for debuggability.

Concurrency and Cancellation

- All long-running operations must be cancellable.
- Use Tokio primitives:
  - cancellation tokens for workflows/tool sessions
  - bounded channels for streaming output
  - timeouts around network and remote execution

Integration Challenges and Guardrails

- Coupling creep: enforce registry/trait boundaries; forbid direct provider/tool types in agent modules
- Dependency bloat: providers/tools behind Cargo features; keep `rustic-ai-core` lean
- Deadlocks/lock contention: minimize shared write locks; prefer message passing for shared context updates
- Plugin safety: versioned API + strict loading errors; document trust model clearly

How to Use This Document

- When adding a feature, identify which layer it belongs to and which boundary it crosses.
- If a change requires multiple layers to know a new detail, stop and redesign the boundary.
- Keep this file aligned with the code as the integration reference.

Requirements Inputs

- `docs/initial-planning/REQUIREMENTS.md` is the capability and quality baseline.
- `docs/initial-planning/tools.md` is the target tool inventory.
- Gaps should be tracked explicitly in planning docs or ADRs.
