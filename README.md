# Rustic-AI

Rustic-AI is a Rust-first, library-centric AI orchestration runtime with a CLI frontend.

- Core engine: `rustic-ai-core` (UI-agnostic runtime, providers, tools, workflows, permissions, storage)
- Frontend: `frontend/rustic-ai-cli` (interactive REPL + operational CLI)

## Project Status

Rustic-AI is beyond the scaffold stage. Core runtime capabilities are broadly implemented, including advanced retrieval for large codebases.

Implemented highlights:
- 7 LLM providers (OpenAI, Anthropic, Google, Grok, Z.ai, Ollama, Custom OpenAI-compatible)
- Broad tooling: shell, filesystem, http, ssh, git, grep, database, web_search, download, regex, format, encoding, convert, lsp, image, mcp, skill, sub_agent
- Workflow engine with rich control flow (tool/skill/agent/workflow/condition/wait/loop/merge/switch)
- Permission model with allow/deny/ask and read-only vs read-write agent enforcement
- Session persistence and storage abstraction (SQLite + Postgres support)
- Learning subsystem (feedback, mistake patterns, preferences, success patterns)
- Big-codebase support (indexing, vector search, hybrid retrieval, graph and impact analysis)

Current major gaps:
- Test coverage and CI quality gates
- API server runtime path and release hardening
- TUI frontend and additional UX polish

See `TODO.md` for the active roadmap.

## Quick Start

### Prerequisites

- Rust toolchain (`rustup` recommended)
- Linux/macOS shell (Windows support is planned through normal Rust tooling and CLI usage)
- Native build deps for TLS/sqlite ecosystems

Debian/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev
```

Install Rust:

```bash
curl https://sh.rustup.rs -sSf | sh
export PATH="$HOME/.cargo/bin:$PATH"
```

### Build

```bash
cargo build --workspace
```

### Configure

```bash
cp config.example.json config.json
export OPENAI_API_KEY="<your-key>"
```

Validate configuration:

```bash
cargo run -p rustic-ai-cli -- --config config.json validate-config --strict
```

### Run CLI

Start a session:

```bash
cargo run -p rustic-ai-cli -- --config config.json session start --agent coder
```

Continue a session:

```bash
cargo run -p rustic-ai-cli -- --config config.json session continue <session-id>
```

### Useful Commands

Workflows:

```bash
cargo run -p rustic-ai-cli -- --config config.json workflow run <workflow-name>
```

Feedback:

```bash
cargo run -p rustic-ai-cli -- --config config.json feedback --type positive --rating 1 --comment "worked well"
```

Indexing / retrieval diagnostics:

```bash
cargo run -p rustic-ai-cli -- --config config.json index status
cargo run -p rustic-ai-cli -- --config config.json index build
cargo run -p rustic-ai-cli -- --config config.json index retrieve --query "permission policy"
cargo run -p rustic-ai-cli -- --config config.json index impact --symbol "ToolManager::execute_tool"
```

## Feature Overview

### LLM Providers

- OpenAI
- Anthropic
- Google (Gemini)
- xAI (Grok)
- Z.ai
- Ollama
- Custom OpenAI-compatible

### Tooling

Built-in tools and adapters include:

- shell, filesystem, http, ssh
- git, grep, database
- web_search, download
- regex, format, encoding, convert
- lsp, image
- mcp, skill, sub_agent

Tool execution supports permission mediation, streaming output, and pending-resolution flows.

### Workflow Engine

Supported step kinds:
- Tool
- Skill
- Agent
- Workflow
- Condition
- Wait
- Loop
- Merge
- Switch

Implemented workflow capabilities:
- grouped conditions
- expression parsing/evaluation
- retry/timeout policies
- routing by success/failure branches

### Permissions and Safety

- policy modes: allow / deny / ask
- persistent decision handling
- command/path pattern controls
- read-only vs read-write agent execution mode

### Learning and Adaptation

- explicit and implicit feedback capture
- mistake pattern detection + warnings
- preference tracking/application
- success pattern library

### Big-Codebase Support

- multi-language symbol indexing (tree-sitter based)
- call graph edge extraction
- vector storage + cosine similarity search
- hybrid retrieval (keyword + vector)
- token-budgeted context injection
- graph/impact diagnostics from CLI

## Repository Layout

```text
.
├── rustic-ai-core/                 # Core runtime library
├── frontend/
│   └── rustic-ai-cli/              # CLI frontend
├── docs/
│   ├── DESIGN_GUIDE.md
│   ├── DECISIONS.md
│   ├── config.schema.json
│   ├── comprehensive-state-analysis.md
│   └── initial-planning/
├── TODO.md                         # Active implementation roadmap
├── AGENTS.md                       # Repo guide for coding agents
└── README.md
```

## Development Workflow

From repo root:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Reference commands and constraints:
- `AGENTS.md`
- `docs/DESIGN_GUIDE.md`
- `TODO.md`

## Documentation

Start with:
- `docs/DESIGN_GUIDE.md`
- `docs/DECISIONS.md`
- `TODO.md`

Planning and requirements:
- `docs/initial-planning/big-picture.md`
- `docs/initial-planning/integration-plan.md`
- `docs/initial-planning/REQUIREMENTS.md`
- `docs/initial-planning/tools.md`

Current-state analysis:
- `docs/comprehensive-state-analysis.md`

## License

TBD.
