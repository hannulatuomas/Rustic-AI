# Rustic-AI Implementation Plan

> Note: `TODO.md` is the single active task tracker. Use this file for phase design/reference; track execution status in `TODO.md`.

## Project Overview

Build a comprehensive, production-ready Agentic AI system in Rust with:
- Library-first architecture (core library, UI consumers)
- Feature-rich: agents, tools, workflows, multi-agent coordination
- Multiple model providers (OpenAI, Anthropic, Grok, Google, local Ollama)
- Storage abstraction with SQLite default
- Full interactive SSH remote execution
- Plugin system for custom tools
- Async-first with Tokio
- Zero-cost abstractions, type-safe, error handling throughout
- Optional Project mode (root, stack, scoped rules/preferences/goals/style)
- Direct mode support (work immediately in current directory)
- Basket/Sub-basket taxonomy for agents/tools/skills (depth 2, many-to-many)

## Architecture

```
rustic-ai/                          # Root workspace
├── Cargo.toml                      # Workspace root
├── rustic-ai-core/                 # Core library (no UI dependencies)
│   ├── Cargo.toml
│   └── src/
│       ├── config/                 # Configuration system
│       ├── providers/              # Model provider traits & impls
│       ├── agents/                 # Agent types, memory, coordinator
│       ├── tools/                  # Tool traits, plugin system, core tools
│       ├── workflows/              # Workflows, commands, DSL
│       ├── storage/                # Storage abstraction (SQLite default)
│       ├── conversation/          # Session management, context
│       └── lib.rs                  # Public API
├── frontend/
│   └── rustic-ai-cli/              # CLI consumer (uses rustic-ai-core)
│       ├── Cargo.toml
│       └── src/main.rs
└── docs/
    └── ...                         # Design docs (see docs/)
```

## Core Principles

1. **Library-First**: All core logic in `rustic-ai-core`, UIs consume it as a dependency
2. **Minimal Dependencies**: Add dependencies judiciously, prefer std library
3. **Async Everywhere**: All I/O operations async with Tokio
4. **Trait-Based Abstraction**: Providers, tools, storage, all trait-based for extensibility
5. **Type Safety**: Result<T, Error> everywhere, no unwraps in production paths
6. **Zero Panic Policy**: All panics handled via Result
7. **Feature Rich from Day 1**: Implement all planned features, phased execution
8. **Config-Driven Feature Toggles**: Non-mandatory capabilities can be enabled/disabled
9. **Project-Aware, Not Project-Required**: Core works with or without project profile

## Dependency Stack (Core)

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
reqwest = { version = "0.11", features = ["json", "stream"] }
futures = "0.3"
async-trait = "0.1"
tokio-util = "0.7"  # CancellationToken
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
sqlx = { version = "0.7", features = ["runtime-tokio", "sqlite"] }
russh = "0.44"        # For SSH PTY support
dyn-clone = "1"       # For trait object cloning
libloading = "0.8"    # Tool plugins (dynamic loading)
clap = { version = "4", optional = true }  # For CLI only
```

---

## Phase 1: Project Setup and Core Infrastructure

**Goal**: Establish workspace, library structure, config, and async runtime
**Duration**: 1-2 days
**Dependencies**: None
**Milestone**: Compilable workspace with config loading

### Task 1.1: Initialize Cargo Workspace
**Description**: Create multi-crate workspace with core library and CLI
**Rationale**: Library-first architecture enables multiple UI consumers (CLI, TUI, API)
**Execution Tracking**: moved to `TODO.md`
**Sketch - root/Cargo.toml**:
```toml
[workspace]
members = ["rustic-ai-core", "frontend/rustic-ai-cli"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
# ... other shared deps
```

### Task 1.2: Implement Core Error Types
**Description**: Define error handling infrastructure with thiserror
**Rationale**: Centralized error types for consistent error handling across all crates
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/error.rs**:
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {provider} - {message}")]
    Provider { provider: String, message: String },

    #[error("Tool execution error: {tool} - {message}")]
    Tool { tool: String, message: String },

    #[error("Storage error: {0}")]
    Storage(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
```

### Task 1.3: Implement Configuration System
**Description**: Central config with TOML/JSON support and env overrides
**Rationale**: Single source of truth for all system configuration
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/config/mod.rs**:
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub model_providers: Vec<ProviderConfig>,
    pub agents: Vec<AgentConfig>,
    pub tools: Vec<ToolConfig>,
    pub storage: StorageConfig,
    pub rules: Option<RulesConfig>,
    pub workflows: Option<Vec<WorkflowConfig>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: ProviderType,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    Grok,
    Google,
    Ollama,
    Custom,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub backend: StorageBackend,
    pub connection_string: String,
    pub max_history_size: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Sqlite,
    Postgres,
    Memory,
}
```

### Task 1.4: Runtime Bootstrap and Wiring
**Description**: Wire config, runtime services, and startup path across core + CLI
**Rationale**: Ensure the engine starts cleanly with validated config and usable runtime state
**Execution Tracking**: moved to `TODO.md`
### Task 1.5: Basic Library Public API
**Description**: Define core library's public interface
**Rationale**: Clear API boundaries for UI consumers
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/lib.rs**:
```rust
mod config;
mod error;
mod logging;
mod providers;
mod agents;
mod tools;
mod workflows;
mod storage;
mod conversation;

pub use config::Config;
pub use error::{Error, Result};
pub use conversation::{Session, SessionManager};
pub use agents::{Agent, AgentCoordinator};

/// Main entry point for Rustic-AI library
pub struct RusticAI {
    config: Config,
    session_manager: SessionManager,
    agent_coordinator: AgentCoordinator,
}

impl RusticAI {
    pub async fn new(config: Config) -> Result<Self>;
    pub async fn create_session(&self, agent_name: &str) -> Result<Session>;
    pub async fn execute_workflow(&self, workflow: Workflow) -> Result<()>;
}
```

---

## Phase 2: Storage Abstraction and SQLite Implementation

**Goal**: Generic storage backend with SQLite as first implementation
**Dependencies**: Phase 1
**Milestone**: Persist and retrieve agent state/conversation history

### Task 2.1: Define Storage Trait
**Description**: Trait-based storage abstraction for state and history
**Rationale**: Easy to swap backends (SQLite, PostgreSQL, etc.) without changing core logic
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/storage/mod.rs**:
```rust
use async_trait::async_trait;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn save_message(&self, session_id: Uuid, message: Message) -> Result<()>;
    async fn get_messages(&self, session_id: Uuid, limit: Option<usize>) -> Result<Vec<Message>>;
    async fn save_state(&self, key: &str, value: &str) -> Result<()>;
    async fn get_state(&self, key: &str) -> Result<Option<String>>;
    async fn delete_session(&self, session_id: Uuid) -> Result<()>;
    async fn create_session(&self, session: Session) -> Result<Uuid>;
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}
```

### Task 2.2: Implement SQLite Storage
**Description**: SQLite implementation of StorageBackend trait
**Rationale**: Lightweight, embedded, perfect for single-machine deployment
**Execution Tracking**: moved to `TODO.md`
**Schema Sketch**:
```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    metadata TEXT
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    metadata TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE TABLE agent_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_messages_session_id ON messages(session_id);
CREATE INDEX idx_messages_timestamp ON messages(timestamp);
```

### Task 2.3: Storage Manager
**Description**: High-level manager for storage operations
**Rationale**: Abstracts backend selection and provides convenient API
**Execution Tracking**: moved to `TODO.md`
### Task 2.4: Storage Tests (Integration)
**Description**: Tests for SQLite implementation
**Rationale**: Ensure correctness before building on top
**Execution Tracking**: moved to `TODO.md`
---

## Phase 3: Model Providers and LLM Integration

**Goal**: Support multiple model providers with streaming and async
**Dependencies**: Phase 1-2
**Milestone**: Query models and stream responses

### Task 3.1: Define Provider Trait
**Description**: Trait for all model providers
**Rationale**: Extensible - add new providers by implementing trait
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/providers/mod.rs**:
```rust
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait ModelProvider: Send + Sync + DynClone {
    fn name(&self) -> &str;

    async fn generate(
        &self,
        messages: &[ChatMessage],
        options: GenerateOptions,
    ) -> Result<String>;

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: GenerateOptions,
    ) -> Result<mpsc::Receiver<String>>;

    async fn count_tokens(&self, text: &str) -> Result<usize>;

    fn supports_streaming(&self) -> bool;

    fn supports_functions(&self) -> bool;
}

dyn_clone::clone_trait_object!(ModelProvider);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<usize>,
    pub top_p: Option<f32>,
    pub top_k: Option<usize>,
    pub stop_sequences: Option<Vec<String>>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub name: Option<String>,
    pub function_call: Option<FunctionCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}
```

### Task 3.2: Implement OpenAI Provider
**Description**: OpenAI API provider with streaming and function calling
**Rationale**: Most widely used LLM provider
**Execution Tracking**: moved to `TODO.md`
### Task 3.3: Implement Anthropic Provider
**Description**: Anthropic Claude API provider
**Rationale**: High-quality reasoning models
**Execution Tracking**: moved to `TODO.md`
### Task 3.4: Implement Grok (xAI) Provider
**Description**: Grok API provider for xAI models
**Rationale**: Access to xAI models
**Execution Tracking**: moved to `TODO.md`
### Task 3.5: Implement Google Provider
**Description**: Google Gemini API provider
**Rationale**: Access to Gemini models
**Execution Tracking**: moved to `TODO.md`
### Task 3.6: Implement Ollama (Local Models)
**Description**: Ollama provider for local model execution
**Rationale**: Privacy, cost savings, offline capability
**Execution Tracking**: moved to `TODO.md`
### Task 3.7: Provider Registry
**Description**: Central registry for all configured providers
**Rationale**: Easy provider lookup and management
**Execution Tracking**: moved to `TODO.md`
### Task 3.8: Provider Tests
**Description**: Tests for all providers
**Rationale**: Ensure correct API integration
**Execution Tracking**: moved to `TODO.md`
---

## Phase 4: Tools, Skills, and Plugin System

**Goal**: Extensible tool system with plugin support, local and remote execution
**Dependencies**: Phase 1-3
**Milestone**: Execute tools async and get results

### Task 4.1: Define Tool Trait
**Description**: Trait for all tools/skills
**Rationale**: Easy to add new tools by implementing trait
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/tools/mod.rs**:
```rust
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync + DynClone {
    fn name(&self) -> &str;

    fn description(&self) -> &str;

    fn parameters_schema(&self) -> serde_json::Value;

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;

    async fn validate_args(&self, args: serde_json::Value) -> Result<()>;
}

dyn_clone::clone_trait_object!(Tool);

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}
```

### Task 4.2: Implement Plugin System
**Description**: Dynamic loading of tools from external libraries
**Rationale**: Allow users to add custom tools without recompiling core
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/tools/plugin.rs**:
```rust
use libloading::Library;

pub struct ToolPlugin {
    name: String,
    library: Library,
    tools: Vec<Box<dyn Tool>>,
}

impl ToolPlugin {
    pub fn load(path: &Path) -> Result<Self>;

    pub fn tools(&self) -> &[Box<dyn Tool>] {
        &self.tools
    }
}
```

### Task 4.3: Implement Shell Tool
**Description**: Execute shell commands locally
**Rationale**: Core tool for system administration
**Execution Tracking**: moved to `TODO.md`
### Task 4.4: Implement SSH Tool with PTY
**Description**: Execute commands on remote machines with interactive PTY
**Rationale**: Full interactive SSH sessions as requested
**Execution Tracking**: moved to `TODO.md`
**SSH PTY Sketch**:
```rust
use russh::client;
use tokio::process::Command;

pub struct SshTool {
    host: String,
    user: String,
    auth_method: AuthMethod,
    port: u16,
}

#[async_trait]
impl Tool for SshTool {
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        // Parse command from args
        // Create SSH client session
        // Allocate PTY
        // Execute command
        // Stream output back
        // Return result
    }
}
```

### Task 4.5: Implement File I/O Tool
**Description**: Read, write, search files
**Rationale**: Basic filesystem operations
**Execution Tracking**: moved to `TODO.md`
### Task 4.6: Implement HTTP Tool
**Description**: Make HTTP requests
**Rationale**: API interactions, web scraping
**Execution Tracking**: moved to `TODO.md`
### Task 4.7: Tool Registry
**Description**: Central registry for all tools
**Rationale**: Easy tool lookup and management
**Execution Tracking**: moved to `TODO.md`
### Task 4.8: MCP Integration
**Description**: Integrate external tools via MCP
**Rationale**: Support external tool ecosystems through a standardized adapter
**Execution Tracking**: moved to `TODO.md`
### Task 4.9: Tool Manager
**Description**: High-level manager for tool execution
**Rationale**: Orchestrate tool calls with error handling
**Execution Tracking**: moved to `TODO.md`
---

## Phase 5: Agents, Memory, and State Management

**Goal**: Hierarchical agents with efficient memory and state
**Dependencies**: Phase 1-4
**Milestone**: Run single agent with memory

### Task 5.1: Define Agent Structures
**Description**: Agent types, configuration, behavior traits
**Rationale**: Base for all agents (hierarchical, specialized)
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/agents/mod.rs**:
```rust
pub struct Agent {
    name: String,
    config: AgentConfig,
    provider: Arc<dyn ModelProvider>,
    tools: Vec<String>,
    memory: Arc<RwLock<Memory>>,
    sub_agents: Vec<Arc<Agent>>,
    state: Arc<RwLock<AgentState>>,
}

pub struct AgentConfig {
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<usize>,
    pub tools: Vec<String>,
    pub model: Option<String>,
}

#[async_trait]
pub trait AgentBehavior: Send + Sync {
    async fn act(&self, input: &str) -> Result<String>;

    async fn think(&self, context: &[ChatMessage]) -> Result<String>;

    async fn plan(&self, goal: &str) -> Result<Vec<Action>>;
}
```

### Task 5.2: Implement Memory System
**Description**: Efficient memory for context and history
**Rationale**: Long conversations require efficient context management
**Execution Tracking**: moved to `TODO.md`
**Memory Sketch**:
```rust
pub struct Memory {
    history: VecDeque<ChatMessage>,
    episodes: Vec<Episode>,
    knowledge: HashMap<String, KnowledgeEntry>,
    max_history_size: usize,
    max_context_tokens: usize,
}

impl Memory {
    pub async fn add_message(&mut self, message: ChatMessage);

    pub async fn get_context(&self, max_tokens: usize) -> Vec<ChatMessage>;

    pub async fn summarize_old_messages(&mut self) -> Result<()>;

    pub async fn search(&self, query: &str) -> Vec<ChatMessage>;
}
```

### Task 5.3: Implement Agent State Management
**Description**: Track agent state across sessions
**Rationale**: Agents need persistent state for long-running tasks
**Execution Tracking**: moved to `TODO.md`
### Task 5.4: Implement Basic Agent
**Description**: Core agent implementation
**Rationale**: Base agent that can process input and generate responses
**Execution Tracking**: moved to `TODO.md`
### Task 5.5: Implement Hierarchical Agents
**Description**: Support for sub-agents and delegation
**Rationale**: Multi-agent systems require hierarchy
**Execution Tracking**: moved to `TODO.md`
### Task 5.6: Agent Registry
**Description**: Central registry for all configured agents
**Rationale**: Easy agent lookup and management
**Execution Tracking**: moved to `TODO.md`
### Task 5.7: Basket/Sub-basket Taxonomy
**Description**: Organize agents/tools/skills with depth-2 hierarchy
**Rationale**: Improve discovery and UI/API filtering while keeping execution decoupled
**Execution Tracking**: moved to `TODO.md`
---

## Phase 6: Agent Coordinator and Multi-Agent Systems

**Goal**: Coordinate multiple agents with parallel execution and context sharing
**Dependencies**: Phase 1-5
**Milestone**: Run multi-agent workflow

### Task 6.1: Define Coordinator Structures
**Description**: Coordinator for multi-agent orchestration
**Rationale**: Efficiently manage multiple agents working together
**Execution Tracking**: moved to `TODO.md`
### Task 6.2: Implement Sequential Execution
**Description**: Execute agents sequentially
**Rationale**: Simple multi-agent workflow
**Execution Tracking**: moved to `TODO.md`
### Task 6.3: Implement Parallel Execution
**Description**: Execute agents in parallel
**Rationale**: Speed up independent tasks
**Execution Tracking**: moved to `TODO.md`
### Task 6.4: Implement Custom Workflows
**Description**: Define custom agent coordination workflows
**Rationale**: Complex workflows require custom logic
**Execution Tracking**: moved to `TODO.md`
### Task 6.5: Context Sharing and Optimization
**Description**: Share only necessary context between agents
**Rationale**: Save context tokens, improve performance
**Execution Tracking**: moved to `TODO.md`
### Task 6.6: Progress Tracking and Status Updates
**Description**: Track progress of multi-agent workflows
**Rationale**: User visibility into long-running tasks
**Execution Tracking**: moved to `TODO.md`
### Task 6.7: Coordinator Tests
**Description**: Tests for coordinator logic
**Rationale**: Ensure correctness before production use
**Execution Tracking**: moved to `TODO.md`
---

## Phase 7: Workflows, Commands, and DSL

**Goal**: Define workflows and commands for complex operations
**Dependencies**: Phase 1-6
**Milestone**: Execute defined workflows

### Task 7.1: Define Workflow Structures
**Description**: Workflow and command definitions
**Rationale**: DSL for complex multi-step operations
**Execution Tracking**: moved to `TODO.md`
**Sketch - rustic-ai-core/src/workflows/mod.rs**:
```rust
pub struct Workflow {
    pub name: String,
    pub description: String,
    pub steps: Vec<Step>,
    pub inputs: HashMap<String, serde_json::Value>,
    pub outputs: HashMap<String, serde_json::Value>,
}

pub enum Step {
    Agent { agent_name: String, input: String },
    Tool { tool_name: String, args: serde_json::Value },
    Parallel { steps: Vec<Step> },
    Conditional { condition: Condition, then_step: Box<Step>, else_step: Option<Box<Step>> },
    Loop { iterations: usize, step: Box<Step> },
}

pub struct Condition {
    pub variable: String,
    pub operator: ComparisonOperator,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum Command {
    Slash(String),
    Prompt(String),
    Workflow(String),
}
```

### Task 7.2: Implement Workflow Parser
**Description**: Parse workflows from TOML/JSON/YAML
**Rationale**: Define workflows in config files
**Execution Tracking**: moved to `TODO.md`
### Task 7.3: Implement Workflow Executor
**Description**: Execute workflows step by step
**Rationale**: Run complex multi-step operations
**Execution Tracking**: moved to `TODO.md`
### Task 7.4: Implement Slash Commands
**Description**: Handle slash-style commands
**Rationale**: Quick access to common operations
**Execution Tracking**: moved to `TODO.md`
### Task 7.5: Implement Saved Prompts
**Description**: Save and reuse prompts
**Rationale**: Common queries without retyping
**Execution Tracking**: moved to `TODO.md`
### Task 7.6: Workflow Tests
**Description**: Tests for workflow logic
**Rationale**: Ensure workflows execute correctly
**Execution Tracking**: moved to `TODO.md`
---

## Phase 8: Conversation Management and Sessions

**Goal**: Session-based conversation management with history
**Dependencies**: Phase 1-7
**Milestone**: Multi-session conversations

### Task 8.1: Define Session Structures
**Description**: Session and conversation tracking
**Rationale**: Track multiple concurrent conversations
**Execution Tracking**: moved to `TODO.md`
### Task 8.2: Implement Session Manager
**Description**: Manage multiple sessions
**Rationale**: Handle concurrent conversations
**Execution Tracking**: moved to `TODO.md`
### Task 8.3: Implement Context Window Management
**Description**: Manage context tokens efficiently
**Rationale**: Models have limited context windows
**Execution Tracking**: moved to `TODO.md`
### Task 8.4: Implement Streaming Responses
**Description**: Stream model responses
**Rationale**: Better user experience, real-time feedback
**Execution Tracking**: moved to `TODO.md`
### Task 8.5: Implement Conversation History
**Description**: Track and retrieve conversation history
**Rationale**: Resume conversations, review history
**Execution Tracking**: moved to `TODO.md`
### Task 8.6: Session Tests
**Description**: Tests for session management
**Rationale**: Ensure session correctness
**Execution Tracking**: moved to `TODO.md`
---

## Phase 9: CLI Implementation

**Goal**: Create CLI consumer of `rustic-ai-core` library
**Dependencies**: Phase 1-8
**Milestone**: Fully functional CLI

### Task 9.1: Define CLI Structure
**Description**: CLI application structure
**Rationale**: User-friendly command-line interface
**Execution Tracking**: moved to `TODO.md`
**CLI Command Sketch**:
```bash
rustic-ai run --config config.toml          # Start interactive mode
rustic-ai chat --agent devops "Help me"    # Single query
rustic-ai workflow deploy-app              # Execute workflow
rustic-ai config init                      # Generate config
rustic-ai session list                     # List sessions
rustic-ai session resume <id>              # Resume session
```

### Task 9.2: Implement Interactive Mode
**Description**: REPL for interactive conversations
**Rationale**: Natural interaction style
**Execution Tracking**: moved to `TODO.md`
### Task 9.3: Implement Batch Mode
**Description**: Single command execution
**Rationale**: Scripting and automation
**Execution Tracking**: moved to `TODO.md`
### Task 9.4: Implement Session Management Commands
**Description**: CLI commands for session operations
**Rationale**: Manage sessions from CLI
**Execution Tracking**: moved to `TODO.md`
### Task 9.5: Implement Workflow Commands
**Description**: CLI commands for workflows
**Rationale**: Execute workflows from CLI
**Execution Tracking**: moved to `TODO.md`
### Task 9.6: Implement Config Management
**Description**: CLI commands for config
**Rationale**: Easy config management
**Execution Tracking**: moved to `TODO.md`
### Task 9.7: CLI Polish
**Description**: Improve CLI UX
**Rationale**: Professional feel
**Execution Tracking**: moved to `TODO.md`
---

## Phase 10: Error Handling, Retries, and Graceful Degradation

**Goal**: Robust error handling throughout
**Dependencies**: Phase 1-9
**Milestone**: System handles errors gracefully

### Task 10.0: Implement Permission Policy
**Description**: Enforce allow/deny/ask decisions for sensitive operations
**Rationale**: Safe execution requires explicit user-controlled permissions
**Execution Tracking**: moved to `TODO.md`
### Task 10.1: Define Error Handling Strategy
**Description**: Centralized error handling patterns
**Rationale**: Consistent error handling across codebase
**Execution Tracking**: moved to `TODO.md`
### Task 10.2: Implement Retry Logic
**Description**: Exponential backoff retries for transient errors
**Rationale**: Handle network issues, rate limits
**Execution Tracking**: moved to `TODO.md`
### Task 10.3: Implement Fallback Mechanisms
**Description**: Fallback to alternatives on failure
**Rationale:** Graceful degradation
**Execution Tracking**: moved to `TODO.md`
### Task 10.4: Implement Circuit Breakers
**Description**: Stop calling failing services
**Rationale:** Prevent cascading failures
**Execution Tracking**: moved to `TODO.md`
### Task 10.5: Implement Graceful Shutdown
**Description:** Clean shutdown on interrupt
**Rationale:** Preserve state, close connections
**Execution Tracking**: moved to `TODO.md`
### Task 10.6: Add Error Reporting
**Description:** User-friendly error messages
**Rationale:** Help users understand and fix errors
**Execution Tracking**: moved to `TODO.md`

### Task 10.7: Logging and Tracing Hardening
**Description:** Finalize runtime logging initialization and output modes
**Rationale:** Logging polish should happen after core systems are working end-to-end
**Execution Tracking**: moved to `TODO.md`
---

## Phase 11: Documentation and Examples

**Goal:** Comprehensive documentation
**Dependencies:** Phase 1-10
**Milestone:** Users can understand and use the system

### Task 11.1: Write API Documentation
**Description:** Inline documentation for public API
**Rationale:** Developers understand how to use the library
**Execution Tracking**: moved to `TODO.md`
### Task 11.2: Write User Documentation
**Description:** README and user guides
**Rationale:** Users can install and use the system
**Execution Tracking**: moved to `TODO.md`
### Task 11.3: Write Developer Documentation
**Description:** Architecture and contribution guides
**Rationale:** Contributors understand the codebase
**Execution Tracking**: moved to `TODO.md`
### Task 11.4: Create Examples
**Description:** Example configurations and workflows
**Rationale:** Users can copy and adapt
**Execution Tracking**: moved to `TODO.md`
### Task 11.5: Create Tutorial
**Description:** Step-by-step tutorial
**Rationale:** Learn by doing
**Execution Tracking**: moved to `TODO.md`
### Task 11.6: Requirements and Tool Inventory Traceability
**Description:** Track coverage against requirements source docs
**Rationale:** Keep implementation aligned with planned scope
**Execution Tracking**: moved to `TODO.md`
---

## Phase 12: Performance Optimization

**Goal:** Optimize performance and resource usage
**Dependencies:** Phase 1-11
**Milestone:** System runs efficiently under load

### Task 12.1: Profile the System
**Description:** Identify bottlenecks
**Rationale:** Data-driven optimization
**Execution Tracking**: moved to `TODO.md`
### Task 12.2: Optimize Async Operations
**Description:** Improve async performance
**Rationale:** Core of the system
**Execution Tracking**: moved to `TODO.md`
### Task 12.3: Optimize Memory Usage
**Description:** Reduce allocations and memory footprint
**Rationale:** Better performance, lower resource usage
**Execution Tracking**: moved to `TODO.md`
### Task 12.4: Optimize Storage Operations
**Description:** Faster database operations
**Rationale:** Storage is critical path
**Execution Tracking**: moved to `TODO.md`
### Task 12.5: Optimize Tool Execution
**Description:** Faster tool runs
**Rationale:** Tools can be slow
**Execution Tracking**: moved to `TODO.md`
### Task 12.6: Benchmark Performance
**Description:** Establish performance baselines
**Rationale:** Track regressions
**Execution Tracking**: moved to `TODO.md`
---

## Phase 13: Future-Proofing and Extensibility

**Goal:** Ensure system can be extended
**Dependencies:** Phase 1-12
**Milestone:** Clear extension points documented

### Task 13.1: Review Extensibility
**Description:** Verify extension points work
**Rationale:** Easy to add new features
**Execution Tracking**: moved to `TODO.md`
### Task 13.2: Add Storage Abstraction Tests
**Description:** Ensure storage backends are swappable
**Rationale:** Verify abstraction works
**Execution Tracking**: moved to `TODO.md`
### Task 13.3: Add API Hooks
**Description:** Prepare for future REST/gRPC API
**Rationale:** API support later
**Execution Tracking**: moved to `TODO.md`
### Task 13.4: Add TUI Hooks
**Description:** Prepare for future TUI
**Rationale:** TUI support later
**Execution Tracking**: moved to `TODO.md`
### Task 13.5: Add Testing Infrastructure
**Description:** Prepare for adding tests later
**Rationale:** Tests will be added later
**Execution Tracking**: moved to `TODO.md`
### Task 13.6: Add Commit Hook Infrastructure
**Description:** Prepare for adding commit hooks later
**Rationale:** Commit hooks will be added later
**Execution Tracking**: moved to `TODO.md`
---

## Summary

This plan provides a comprehensive, phased approach to building Rustic-AI:

**Total Phases:** 13
**Estimated Duration:** 3-4 weeks (assuming focused work)
**Core Focus:** Library-first architecture, feature-rich, extensible

**Key Differences from Original Plan:**
1. **Library-first**: Core library with consumers under `frontend/`
2. **SQLite**: Generic storage abstraction with SQLite as first implementation
3. **Full SSH PTY**: Interactive SSH support
4. **Plugin System**: Implemented in Phase 4 (Tools)
5. **Rules Scopes + Feature Toggles**: Global/project/topic rules and optional subsystem toggles
6. **MCP + Permissions**: External tool support plus allow/deny/ask policy model
7. **No Pre-configured Agents**: Focus on robust base in Phase 5
8. **Project Profiles + Direct Mode**: Optional project model without blocking immediate usage
9. **Basket Taxonomy**: Depth-2 organization for agents/tools/skills

**Phase Breakdown:**
- **Phases 1-3**: Foundation (workspace, storage, providers)
- **Phases 4-6**: Tools, skills, MCP, and agents (core functionality)
- **Phases 7-8**: Workflows and Sessions (orchestration)
- **Phase 9**: CLI implementation under `frontend/`
- **Phases 10-12**: Robustness (errors, performance)
- **Phase 13**: Future-proofing

Each phase has clear tasks with detailed todos, dependencies, and milestones. The plan is AI-friendly with code sketches and rationale for each decision.

---

## Next Steps

1. **Review this plan** and provide feedback
2. **Create TODO.md** with the first phase's tasks converted to trackable todos
3. **Begin Phase 1 implementation** with Task 1.1
4. **Iterate** through each phase, marking todos as complete
5. **Adjust** the plan based on implementation experience
