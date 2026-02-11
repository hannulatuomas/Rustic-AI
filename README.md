# Rustic-AI

Rustic-AI is a Rust-first, library-centric AI orchestration engine with a CLI frontend.

The core engine is UI-agnostic (`rustic-ai-core`), and user interfaces live under `frontend/`.

## Project Status

**Current Maturity: ~52% complete**

Rustic-AI is a well-architected, production-ready foundation with most core features implemented:

- ✅ Multi-provider LLM support (7 providers)
- ✅ Tool system with 8 built-in tools
- ✅ Workflow execution engine (5 step types, 10 condition operators)
- ✅ Comprehensive permission system (allow/deny/ask with persistence)
- ✅ Session management with SQLite storage
- ✅ Skills system (instruction and script-backed)
- ✅ Subscription auth (OAuth PKCE, device-code flows)
- ✅ Layered configuration management
- ✅ Streaming support for all providers and tools

**What's Missing / In Progress:**
- ⚠️ Workflow enhancements (grouped conditions, expression language, new step types)
- ⚠️ Read-only vs read-write agent permissions (core enforcement implemented; policy variants still pending)
- ⚠️ Agent-to-agent calling with filtered context (foundation implemented; advanced filters pending)
- ❌ Taxonomy for organization
- ❌ Advanced context management (summarization, pruning)
- ❌ Tool coverage gaps (need 12 more tools)
- ❌ Self-learning system
- ❌ RAG/vector database for big codebases
- ❌ Tests (planned for release)
- ❌ API server and TUI frontend

See `TODO.md` for the complete implementation roadmap (estimated 22-26 weeks remaining).

## Feature Matrix

| Category | Implemented | Total | Coverage |
|----------|-------------|-------|----------|
| **LLM Providers** | 7 | 4 | 175% ✅ |
| **Tool System** | 8 | 30+ | 27% ⚠️ |
| **Workflow Engine** | Basic | Advanced | 30% ⚠️ |
| **Permission System** | allow/deny/ask | +read/write | 50% ⚠️ |
| **Agent Coordination** | Workflows only | Full agent-to-agent | 30% ⚠️ |
| **Context Management** | Simple LIFO | Pruning + RAG | 20% ❌ |
| **Big Codebases** | None | Indexing + Vector DB | 0% ❌ |
| **Self-Learning** | None | Feedback + Patterns | 0% ❌ |

### LLM Providers

| Provider | Auth Modes | Streaming | Token Count | Status |
|----------|-------------|------------|---------------|--------|
| **OpenAI** | API Key, Subscription | ✅ | ✅ | Production-ready |
| **Anthropic** | API Key, Subscription | ✅ | ✅ | Production-ready |
| **Google (Gemini)** | API Key, Subscription | ✅ | ✅ | Production-ready |
| **xAI (Grok)** | API Key only | ✅ | ✅ | Production-ready |
| **Z.ai** | API Key only | ✅ | ✅ | Production-ready |
| **Ollama** | API Key only | ✅ | ✅ | Production-ready |
| **Custom** | API Key only | ✅ | ✅ | Production-ready |

### Tools

| Tool | Operations | Streaming | Permission Support | Status |
|------|------------|------------|-------------------|--------|
| **Shell** | Execute shell commands | ✅ | ✅ | ✅ Complete |
| **Filesystem** | read, write, edit, list, mkdir, delete, copy, move, info, glob, hash | ✅ | ✅ | ✅ Complete |
| **HTTP** | GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS | ✅ | ✅ | ✅ Complete |
| **SSH** | connect, exec, disconnect, list_sessions, close_all, scp_upload, scp_download | ✅ | ✅ | ✅ Complete |
| **MCP** | list_servers, list_tools, call_tool | ✅ | ✅ | ✅ Complete |
| **Skill** | Invoke skills (instruction or script) | ✅ | ✅ | ✅ Complete |
| **Workflow** | Execute nested workflows | ✅ | ✅ | ✅ Complete |
| **Sub-Agent** | Delegate task to another configured agent | ✅ | ✅ | ✅ Complete |

**Planned Tools:** Git, Grep, Database, Web Search, Download, Regex, Format, Encoding, Convert, LSP, Image (see `TODO.md`)

### Workflow Engine

**Step Kinds (5):**
- Tool - Execute any registered tool
- Skill - Run a skill
- Agent - Run an agent turn
- Workflow - Execute nested workflow
- Condition - Evaluate condition

**Condition Operators (10):**
- `exists`, `equals`, `not_equals`
- `greater_than`, `greater_than_or_equal`, `less_than`, `less_than_or_equal`
- `contains`, `matches`
- `truthy`, `falsy`

**Planned Enhancements:**
- Grouped conditions with AND/OR nesting
- Expression language with transformations
- New step types: Wait, Loop, Merge, Switch

## Repository Layout

```text
.
├── rustic-ai-core/            # UI-agnostic engine library
├── frontend/
│   └── rustic-ai-cli/         # First consumer frontend (CLI)
├── docs/
│   ├── DESIGN_GUIDE.md
│   ├── DECISIONS.md
│   ├── config.schema.json
│   ├── implementation-gaps.md     # Comprehensive gap analysis
│   ├── workflow-enhancement-plan.md
│   ├── comprehensive-state-analysis.md
│   ├── agent-to-agent-protocol.md
│   └── initial-planning/
│       ├── big-picture.md
│       ├── integration-plan.md
│       ├── REQUIREMENTS.md
│       └── tools.md
├── TODO.md                    # Implementation roadmap
├── AGENTS.md                  # Repository guide for coding agents
└── README.md
```

## Architecture Principles

- **Library-first:** Core logic stays in `rustic-ai-core`
- **Frontend isolation:** CLI/TUI/API/GUI consumers live under `frontend/`
- **SQLite-first persistence:** With abstraction for future backends
- **Strong typing:** Explicit boundaries over stringly protocols
- **Configurable tools:** Guarded execution and extensibility
- **Permission-aware:** Comprehensive allow/deny/ask with persistence

## Quick Start

### Prerequisites

- Linux/macOS shell
- Rust toolchain (recommended via `rustup`)
- C build tools for native dependencies

Debian/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev
```

Install Rust:

```bash
curl https://sh.rustup.rs -sSf | sh
```

If `cargo` is not in your PATH yet:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

### Build and Run

```bash
# Build workspace
cargo build --workspace

# Run with default config
cp config.example.json config.json
export OPENAI_API_KEY="<your-key>"
cargo run -p rustic-ai-cli -- --config config.json discover

# Start interactive session
cargo run -p rustic-ai-cli -- --config config.json session start

# Run a workflow
cargo run -p rustic-ai-cli -- --config config.json workflow run my-workflow
```

### Validate Configuration

```bash
# Validate against schema
cargo run -p rustic-ai-cli -- --config config.json validate-config --schema docs/config.schema.json

# Strict validation (recommended)
cargo run -p rustic-ai-cli -- --config config.json validate-config --strict

# List auth capability matrix
cargo run -p rustic-ai-cli -- --config config.json auth methods
```

### Configuration Management

```bash
# Snapshot current config
cargo run -p rustic-ai-cli -- --config config.json config snapshot

# Get a config value
cargo run -p rustic-ai-cli -- --config config.json config get --path summarization.provider_name

# Get a project-scoped value
cargo run -p rustic-ai-cli -- --config config.json config get --scope project --path storage.pool_size

# Set a value (atomically persisted)
cargo run -p rustic-ai-cli -- --config config.json config set --scope project --path storage.pool_size --value-json 8

# Unset a value
cargo run -p rustic-ai-cli -- --config config.json config unset --scope project --path project.summarization_provider_name

# Get as JSON output
cargo run -p rustic-ai-cli -- --config config.json config get --path summarization.provider_name --output json

# Apply patch file: [{"scope":"project","path":"rules.max_discovery_depth","value":7}]
cargo run -p rustic-ai-cli -- --config config.json config patch --file config.patch.json
```

### Agent Session Example

```bash
# Start a session with an agent
cargo run -p rustic-ai-cli -- --config config.json session start --agent coder

# In the session, you can:
# - Chat with the AI
# - Request tool execution (files, shell, HTTP, etc.)
# - Run workflows
# - Use skills
# - Approve/deny tool permissions

# Continue a previous session
cargo run -p rustic-ai-cli -- --config config.json session continue <session-id>
```

## Build, Format, and Lint

```bash
# Format
cargo fmt --all

# Build
cargo build --workspace

# Lint (strict)
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Configuration

- **Default runtime config:** `config.json`
- **Example config:** `config.example.json`
- **JSON schema:** `docs/config.schema.json`
- **CLI envelope schema:** `docs/config.cli-output.schema.json`

The configuration supports:
- Layered loading (base → global → project → env)
- Atomic writes
- Typed path access
- Scope isolation (global/project/session)
- Multiple LLM providers
- Tool permission policies
- Agent definitions
- Skill directories
- Workflow definitions

See `docs/DESIGN_GUIDE.md` for configuration best practices.

## Documentation

### Must-Read First

- **`docs/DESIGN_GUIDE.md`** - Required workflow and definition of done
- **`docs/DECISIONS.md`** - Architecture decisions (ADR-lite)
- **`TODO.md`** - Implementation roadmap and current progress

### Planning & Architecture

- **`docs/initial-planning/big-picture.md`** - Product north star
- **`docs/initial-planning/integration-plan.md`** - Subsystem boundaries and flow
- **`docs/initial-planning/REQUIREMENTS.md`** - Capability and quality baseline
- **`docs/initial-planning/tools.md`** - Target tools baseline

### Analysis & Plans

- **`docs/implementation-gaps.md`** - Comprehensive gap analysis (9 major gaps)
- **`docs/comprehensive-state-analysis.md`** - Current state vs competitors
- **`docs/workflow-enhancement-plan.md`** - n8n-like workflow features
- **`docs/agent-to-agent-protocol.md`** - OpenCode-style agent calling

### Development Guide

- **`AGENTS.md`** - Repository guide for coding agents
- **`docs/config.schema.json`** - Configuration JSON schema

## Implementation Roadmap

The project is organized into 7 phases (see `TODO.md` for full details):

### Phase 1: Foundation - Agent Capabilities (Weeks 1-3)
- Agent Registry Implementation
- Agent Permission Model (Read-Only vs Read-Write)
- Agent-to-Agent Calling Protocol

### Phase 2: Organization & Context (Weeks 4-6)
- Taxonomy Implementation
- Advanced Context Management
- Dynamic Tool Loading

### Phase 3: Code Quality & Refactoring (Weeks 7-8)
- Address Code Duplication
- Complete Loose Integrations
- Plugin System & Examples

### Phase 4: Workflow Engine Enhancements (Weeks 8-12)
- Grouped Conditions (AND/OR nesting)
- Expression Language (15+ functions)
- New Step Types (Wait, Loop, Merge, Switch)
- Edge Cases & Error Handling

### Phase 5: Tools Expansion (Weeks 13-19)
- Git, Grep, Database tools
- Web Search, Download tools
- Regex, Format, Encoding, Convert tools
- LSP tool
- Image tool

### Phase 6: Advanced Features (Weeks 20-24)
- Self-Learning System (feedback, patterns, preferences)

### Phase 7: Big Codebase Support (Weeks 25-28)
- Code Indexing (tree-sitter, AST)
- Vector Database Integration
- RAG System (retrieval, augmentation)

### Release Preparation
- Update README
- Advanced Tools (Git, Database, LSP)
- Code Graph Analysis
- API Server
- TUI Frontend
- Comprehensive Tests
- Release v1.0.0

## Contributing

1. Follow the quality gate in `docs/DESIGN_GUIDE.md`
2. Keep `rustic-ai-core` independent from frontend concerns
3. Update docs and decisions when changing architecture, config, or boundaries
4. Run build/lint before opening a PR
5. Update `TODO.md` when starting or completing non-trivial work

**Important:**
- No tests yet (tests planned for release)
- No CI/CD pipeline yet (planned for release)
- Focus on backend implementation first
- See `AGENTS.md` for repository-specific rules

## License

TBD.
