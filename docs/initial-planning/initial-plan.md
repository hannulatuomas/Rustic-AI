### Project Overview

As a senior software and AI engineer with over 20 years of experience (spanning systems programming in C/C++, Rust since its early days, AI/ML pipelines at scale, and building extensible agentic systems), this is a comprehensive, AI-friendly plan for your Agentic AI system.

Project name: **Rustic-AI** (engine/library), with UI consumers (CLI/TUI/API/GUI) under `frontend/`.

This plan prioritizes:
- **Rust Best Practices**: Zero-cost abstractions via traits, generics, and enums; async with Tokio; minimal dependencies (e.g., reqwest for HTTP, serde for serialization, tokio for async); no bloat—start with core crates and add judiciously.
- **Extensibility**: Use traits for providers, tools, agents, etc., so adding new ones is as simple as implementing a trait and registering it.
- **Performance**: Async I/O everywhere; efficient memory (e.g., Arc for shared state, RwLock for concurrency); minimal allocations (use Cow, SmallVec where needed).
- **Correctness**: Strong typing, Result/Option everywhere; no panics in production paths.
- **Feature-Rich from Day 1**: No MVP—implement all listed features, but in phases to avoid overwhelm.
- **AI-Friendly Structure**: Clear phases with numbered tasks, sub-tasks (todos), dependencies, and milestones.
- **Library-First**: Core engine is a Rust library that can be embedded in any UI (CLI first, TUI/API later) without polluting the core.
- **Skip Testing/Hooks**: Focus on core; add later.
- **Customizability**: Everything via config files (TOML/JSON), env vars, or runtime overrides.
- **Feature Toggles**: All non-mandatory subsystems must be enable/disable configurable.
- **Project Profiles**: Support direct mode and optional project mode with scoped metadata.

**High-Level Architecture**:
- **Modular Crate Structure**: Workspace with a library-first engine crate (e.g., `rustic-ai-core`) and UI consumer crates under `frontend/` (e.g., `frontend/rustic-ai-cli`).
- **Key Components**:
  - **Config Layer**: Central config struct, loaded from files/env.
  - **Project Layer**: Optional project profile (root path, stack, rules, goals, preferences, styling/design guidance).
  - **Permissions Layer**: Central allow/deny/ask policy with session-aware decisions.
  - **Model Providers**: Trait-based (e.g., `ModelProvider` with methods like `generate`, `stream`).
  - **Agents**: Hierarchical (Agent, SubAgent) with memory, state, tools.
  - **Tools/Skills**: Trait-based executors (local, remote via SSH, and MCP adapters).
  - **Workflows/Commands**: DSL or struct-based for sequences, slash commands as enums.
  - **Workflow Triggers**: Event/schedule/webhook trigger model for future visual workflow UIs.
  - **Taxonomy Layer**: Basket/Sub-basket metadata for organizing agents/tools/skills.
- **Memory/State**: Efficient, bounded in-memory working set with durable persistence via a storage trait (SQLite first; swappable later).
  - **Conversation Manager**: Session-based history with context window management.
  - **Multi-Agent System**: Coordinator with parallel execution (tokio tasks), shared context via channels.
- **Remote Execution**: SSH integration with full interactive PTY support (russh-based).
- **Data Flow**: User input → Conversation Manager → Agent Coordinator → Model/Tool Execution → Response streaming.
- **Dependencies**: Core: tokio, serde, reqwest, thiserror, tracing; sqlx (SQLite); russh (SSH PTY); libloading (tool plugins). Add per-feature behind Cargo features.

**Assumptions**:
- Target: Linux/Mac/Windows (cross-platform via Rust).
- Runtime: First consumer is CLI (e.g., `rustic-ai run --config config.toml`), but core is UI-agnostic.
- Security: No auth yet; assume trusted environment.
- Scale: Designed for single-machine, but extensible to distributed.

**Risks/Mitigations**:
- Complexity: Phase-based to build incrementally.
- Performance Bottlenecks: Profile async paths early.
- LLM Costs: Support rate-limiting, fallbacks.

Now, the phased plan.

### Phase 1: Workspace Setup and Core Infrastructure
**Goal**: Establish workspace, config, logging, and basic async runtime. This is the foundation for extensibility and performance.
**Duration Estimate**: 1-2 days.
**Dependencies**: None.
**Milestones**: Compilable workspace; core library loads config; CLI can bootstrap the engine.

**Tasks**:
1. **Set Up Workspace and Crates (Library-First)**:
   - Create a Cargo workspace.
   - Add engine crate: `rustic-ai-core` (library).
    - Add first consumer: `frontend/rustic-ai-cli` (binary) that depends on `rustic-ai-core`.
   - Optional later: `rustic-ai-tui`, `rustic-ai-server`.
   - Rationale: Keep engine UI-agnostic; UI crates stay thin.
   - Todos:
      - Root `Cargo.toml` workspace members include `rustic-ai-core` and `frontend/rustic-ai-cli`.
     - Prefer `[workspace.dependencies]` to keep versions consistent.

2. **Implement Config System**:
   - In `core`: Define `struct Config` with fields like `model_providers: Vec<ProviderConfig>`, `agents: Vec<AgentConfig>`, `tools: Vec<ToolConfig>`, etc. Use serde::Deserialize.
     - Support rules/context files: `.cursorrules`, `.windsurfrules`, `AGENTS.md`, `CLAUDE.md`, etc.
     - Support scoped rules: global, project, and topic/session.
    - Load from TOML/JSON/env: Fallback chain (file -> env overrides -> runtime overrides).
   - Rationale: Central config for customizability; easy to add fields.
   - Todos:
     - Sketch: `#[derive(Deserialize)] struct Config { /* fields */ }`
     - Function: `fn load_config(path: &str) -> Result<Config>` using toml::from_str or env overrides.
     - Parse rules into enforceable policy artifacts (and also inject into prompts as needed).
     - Add explicit precedence: global -> project -> topic/session -> runtime override.
     - Add feature toggles for non-mandatory subsystems.
     - Add project profile schema and mode selection (direct vs project).

3. **Set Up Logging and Error Handling**:
    - Use `tracing` for structured logs.
    - Central typed error enum in core (thiserror), with `type Result<T> = std::result::Result<T, Error>`.
   - Rationale: Correctness; traceable async errors.
    - Todos:
      - In CLI main: initialize tracing subscriber.
      - In core: define `Error` and propagate context-rich errors.

4. **Basic Async Runtime**:
    - In CLI: Tokio runtime with graceful shutdown.
    - In core: expose cancellation hooks; avoid embedding any terminal/UI assumptions.
   - Rationale: Performance first—async from the start.
   - Todos:
     - `#[tokio::main] async fn main() -> Result<()> { let config = load_config("config.toml")?; /* later phases */ Ok(()) }`

### Phase 2: Storage Abstraction + SQLite (Persistence)

**Goal**: Durable sessions/history/state via a storage trait with SQLite as first backend.
**Dependencies**: Phase 1.
**Milestones**: Create session; append/retrieve messages; persist state; migrations.

**Tasks**:
1. **Define Storage Trait**:
   - In core: `trait StorageBackend` for sessions/messages/state.
   - Rationale: Swap backends later without changing orchestration.

2. **Implement SQLite Backend**:
   - Use `sqlx` with SQLite.
   - Add schema migrations and versioning.
   - Rationale: Embedded, robust, queryable.

3. **Context Window Hooks**:
   - Persist full history; compute context window in memory.
    - Summarization hooks for overflow.

4. **Project Persistence**:
   - Persist project profiles and scoped preferences/decisions.
   - Link sessions optionally to project profiles.

### Phase 3: Model Providers and LLM Integration
**Goal**: Support multiple model providers (APIs and local) with streaming and async.
**Dependencies**: Phase 1-2.
**Milestones**: Can query a model and stream response.

**Tasks**:
1. **Define Provider Trait**:
   - In `providers`: `trait ModelProvider: Send + Sync { async fn generate(&self, prompt: &str, options: GenerateOptions) -> Result<String>; async fn stream_generate(&self, prompt: &str, options: GenerateOptions) -> Result<Receiver<String>>; }` (use tokio::sync::mpsc for streaming).
   - `struct GenerateOptions` for temp, max_tokens, etc.
   - Rationale: Extensible—impl for each provider.
   - Todos:
     - Add retry logic: Exponential backoff with tokio-retry.

2. **Implement API Providers**:
   - For OpenAI, Anthropic, Grok (xAI), Google: Use reqwest for HTTP.
   - Support existing subscriptions: Config fields for API keys.
   - Example: `struct OpenAIProvider { client: reqwest::Client, api_key: String };` impl trait with JSON payloads.
   - Handle streaming: Parse SSE (server-sent events).
   - Rationale: Feature-rich—multi-provider from start.
   - Todos:
     - Add deps: reqwest = { version = "0.11", features = ["json"] }, futures = "0.3".
     - Graceful degradation: Fallback to another provider on error.

3. **Implement Local Models**:
   - For Ollama/llama-cpp: Use crates like ollama-rs or custom subprocess.
   - Config: `local_model_path: PathBuf`.
   - Todos:
     - Add dep: tokio-process if needed.
     - Impl trait similarly, with async pipes for streaming.

4. **Provider Registry**:
   - In providers: `struct ProviderRegistry { providers: HashMap<String, Box<dyn ModelProvider>> };` load from config.
   - Select by name: `fn get_provider(&self, name: &str) -> Option<&dyn ModelProvider>`.
   - Todos: Register in main from config.

### Phase 4: Tools, Skills, MCP, Plugins, and Execution (Local + Remote)
**Goal**: Build extensible tools with local/remote execution.
**Dependencies**: Phase 1-3.
**Milestones**: Execute a tool async and get results.

**Tasks**:
1. **Define Tool and Skill Traits**:
   - In `tools`: `trait Tool: Send + Sync { fn name(&self) -> &str; async fn execute(&self, args: &str) -> Result<String>; }` (args as JSON string for flexibility).
   - Support skills as first-class components (instruction-only or script-backed).
   - Rationale: Easy to add—impl and register.
   - Todos: Error handling with retries.

2. **Implement Core Tools (Built-in)**:
   - Basic: Shell exec (tokio::process::Command), file I/O.
    - Remote: SSH with full interactive PTY support (russh). Support stdin/stdout streaming, terminal resize, cancellation, host key policy.
   - Use-case specific: E.g., Docker tool for container ops, SQL tool for DB (via sqlx later if needed).
   - Rationale: Cover use cases like DevOps, cyber sec (e.g., nmap via shell).
    - Todos:
      - Use russh for interactive PTY.
     - Config: Tool configs with params.

3. **Tool Registry**:
   - Similar to providers: HashMap of Box<dyn Tool>.
    - Load from config; allow custom tools via plugins (dynamic loading) early.

4. **Plugin System (Tools First)**:
    - Dynamic tool loading via `libloading`.
    - Versioned plugin manifest/API compatibility checks.
    - Rationale: Extensibility without recompiling.

5. **MCP Integration**:
    - Add MCP adapter layer to expose external tools through the same Tool contract.
    - Config toggle for MCP and per-server/tool controls.

6. **Permission Policy Integration**:
    - Enforce allow/deny/ask for sensitive tool actions.
    - Ask decisions support: allow once, allow for session, deny.

7. **Integration with Models**:
   - Tools in prompts: Function calling style (e.g., JSON schema for tools).
   - Parse model output for tool calls, execute async.

### Phase 5: Agents and Multi-Agent Systems
**Goal**: Hierarchical agents with efficiency.
**Dependencies**: Phase 1-4.
**Milestones**: Run a multi-agent workflow.

**Tasks**:
1. **Define Agent Structures**:
   - In `agents`: `struct Agent { name: String, provider: String, tools: Vec<String>, memory: Arc<RwLock<Memory>>, sub_agents: Vec<Arc<Agent>> };`.
   - Sub-agents: Recursive.
   - Rationale: Hierarchical for multi-agent.
   - Todos: Traits for behavior: `trait AgentBehavior { async fn act(&self, input: &str) -> Result<String>; }`.
   - Add rich agent config (provider/model, tool/skill sets, retry policy, limits, permissions profile).

2. **Memory and State Management**:
   - `struct Memory { history: Vec<Message>, context: HashMap<String, String> };` (efficient: limit size, summarize old context).
    - Persistence: Use storage trait (SQLite backend) for sessions/history/state.
   - State: Similar, with sessions (UUID-keyed).
   - Rationale: Efficient for long convos—prune/share only necessary.
   - Todos: Async read/write; compression if needed.

3. **Agent Coordinator**:
   - `struct Coordinator { agents: Vec<Arc<Agent>> }; async fn execute_workflow(&self, workflow: Workflow, input: &str) -> Result<()>;`.
   - Parallel: Spawn tokio tasks, join with channels for shared context.
   - Progress: Use mpsc for status updates.
   - Rationale: Handle multi-agent efficiently.

4. **Support Use Cases**:
   - Pre-config agents: E.g., DevOpsAgent with Docker tools, CyberAgent with nmap/ethical tools.
    - Customizable via config.

5. **Basket/Sub-basket Taxonomy**:
   - Add basket hierarchy (depth 2) for agents/tools/skills.
   - Allow many-to-many membership for discovery/filtering.

### Phase 6: Workflows, Commands, and Conversation Management
**Goal**: Glue everything with workflows and sessions.
**Dependencies**: All prior.
**Milestones**: Full system runnable via CLI.

**Tasks**:
1. **Workflows and Commands**:
   - In `workflows`: `enum Command { Slash(String), Prompt(String) }; struct Workflow { steps: Vec<Step> }; enum Step { Agent(String), Tool(String), Parallel(Vec<Step>) };`.
   - Saved prompts: Config vec; slash commands can map to workflows or prompt templates.
   - Add workflow triggers (manual/schedule/webhook/event) as first-class metadata.
   - Rationale: DSL-like for sequences.
   - Todos: Parser for rules/context files into workflows.

2. **Conversation Manager**:
   - `struct ConvManager { sessions: HashMap<Uuid, Session> }; struct Session { history: Vec<Message>, state: State };`.
   - Track history, manage context (truncate/summarize).
   - Async streaming responses.

3. **Error Handling and Degradation**:
   - Retries: In all async calls.
   - Fallbacks: E.g., switch providers on quota error.

4. **Integration and Entry Point**:
   - In main: Load config, registries; CLI loop or single run.
    - Support all features: Rules as workflow modifiers, etc.

5. **Project Mode and Direct Mode**:
   - Support running without project setup (direct mode).
   - Support optional project mode with project root/stack/scoped guidance.

6. **Requirements and Tool Inventory Alignment**:
   - Track implementation coverage against `docs/initial-planning/REQUIREMENTS.md`.
   - Track tool coverage against `docs/initial-planning/tools.md`.

### Phase 7: Polish and Extensibility Checks
**Goal**: Ensure feature-complete, performant.
**Dependencies**: All.
**Milestones**: Demo multi-agent use case (e.g., DevOps script).

**Tasks**:
 1. **Review Extensibility**: Add example-new provider/tool/agent via config, plus a tool plugin.
2. **Performance Optimizations**: Profile allocations, async bottlenecks.
3. **Documentation**: In-code docs; README with config examples.
 4. **Future Hooks**: API/TUI/GUI layers under `frontend/`, including visual workflow builders.

This plan is exhaustive yet phased—implement one phase at a time with OpenCode. Each task is granular for AI delegation (e.g., "Implement OpenAIProvider trait"). If needed, refine based on progress. Let's build something exceptional!
