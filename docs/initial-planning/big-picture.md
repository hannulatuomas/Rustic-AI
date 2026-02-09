Big Picture Plan

This is the "north star" for the project: what we are building, why it exists, what principles are non-negotiable, and what success looks like. It is intentionally UI-agnostic: the core is a Rust library that can be embedded in a CLI/TUI today and an API tomorrow, without rewriting the engine.

Quality Gate

We follow a strict workflow to keep Rustic-AI clean, correct, and consistent:

- Required loop: plan -> implement -> quality check/validation -> fix all issues properly -> validate against big-picture/integrations -> update TODO
- No partial implementations, no shortcuts, no deferring fixes, no documentation drift
- Documentation updates ship with the code changes they describe
- `TODO.md` is the single active task tracker and must be updated in every non-trivial change

Source of truth: `docs/DESIGN_GUIDE.md`

Main Idea

Rustic-AI is a high-performance, extensible agentic AI engine written in Rust. It orchestrates agents and sub-agents, integrates multiple model providers, executes tools locally and remotely (including fully interactive SSH PTY sessions), and runs workflows/slash commands with robust state and conversation management.

It supports two operating modes:
- Direct mode: start in current folder and work immediately (Claude Code/OpenCode style).
- Project mode: start with a defined Project profile (root path, tech stack, scoped rules, goals, decisions/preferences, design/styling guidance).

Metaphor: A "conductor and orchestra".
- Coordinator: the conductor (schedules, routes, applies policies)
- Agents: the musicians (reason, plan, delegate)
- Tools: the instruments (shell, filesystem, ssh, http, custom plugins)
- Workflows: the score (typed steps, parallelism, conditionals)
- Memory/State: sheet music + rehearsal notes (context windowing, summaries, persistence)

Non-Goals (At First)

- No hard commitment to a specific UI (CLI is first consumer; TUI/API later)
- No heavy test/hook infrastructure initially (we will add later)
- No distributed multi-node execution initially (design for it, don't implement it yet)

Goals

Primary Goal

Build a feature-rich, non-MVP core engine that can match and exceed capabilities of tools like Claude Code/OpenCode: agents, rules/context, workflows/commands, tool use, multi-provider models, streaming, and reliable long-running sessions.

Short-Term Goals (First Iterations)

- Compile and run a minimal but real engine: config -> registries -> sessions -> agent -> provider -> tool calls -> persistence
- Establish stable public library APIs (so UI layers stay thin)
- End-to-end demo: a session that uses at least one provider + one tool + stores history/state

Long-Term Goals (3-6 Months+)

- Extensibility without recompiling: tools (and later providers/agents) via plugins
- Strong integration story: clean boundaries, feature flags, migration/versioning
- Mature capabilities: multi-agent coordination, efficient context sharing, progress/events, safe remote ops

Success Metrics

- Extensibility: add a new tool in one file + config entry; add a new provider behind a trait without touching agents
- Maintainability: new features mostly add code, not churn existing core; clear separation of crates/modules
- Reliability: transient failures recover via retries/fallback; sessions recover after restart
- Performance: parallel agents and tool runs without blocking; bounded memory growth; efficient context windows
- Safety: no panics in core paths; no unsafe by default; remote execution includes host key policies and timeouts

Core Principles (Non-Negotiable)

Performance First

- Async I/O everywhere (Tokio)
- Avoid unnecessary allocations; bounded queues; careful locking (minimize write-lock scope)
- Prefer typed enums/structs over stringly-typed protocols

Correctness and Safety

- Explicit error types with context; Result everywhere; no unwrap/expect in core
- Deterministic behavior: config-driven defaults; versioned schemas; reproducible workflows

Extensibility and Clean Boundaries

- Trait-based interfaces for providers/tools/storage
- Registry pattern for runtime wiring (no direct coupling between agents and provider implementations)
- Feature flags to avoid dependency bloat (e.g., enable only the providers/tools you need)

Library-First, UI-Agnostic

- `rustic-ai-core` is the engine: no CLI parsing, no terminal concerns
- UI layers (CLI/TUI/API/GUI) are thin adapters that translate events and I/O
- UI consumers live under `frontend/` (e.g., `frontend/rustic-ai-cli`)

Efficient Context and Long-Running Sessions

- Context windows are managed explicitly (truncate/summarize/retain key messages)
- Share only necessary context between agents (filtered/summarized) to reduce token pressure
- Durable sessions: history + state persisted via a storage backend abstraction

Key Features

Engine Capabilities

- Agents/sub-agents and multi-agent coordination (sequential + parallel)
- Rules and context files (.cursorrules/.windsurfrules, AGENTS.md/CLAUDE.md style) as first-class inputs
- Rule scopes: global, project, and topic/session level with explicit precedence
- Optional Project profiles with project-scoped configuration and lifecycle
- Workflows, slash commands, saved prompts; typed step execution with parallelism/conditionals
- Permissions are policy-driven: allow, deny, ask (allow once, allow in session, deny)
- Tool integration: local execution + remote execution (interactive SSH PTY)
- Skills as first-class building blocks (instruction files and script-backed skills)
- MCP integration for external tool ecosystems
- Progress/status events as a stream (for CLI/TUI/UI integrations)
- Future-ready workflow graph and trigger model for rich n8n-style workflow UIs
- Basket taxonomy for discoverability and UX: Basket -> Sub-basket (depth 2), many-to-many item membership

Model Providers

- Multiple providers (OpenAI, Anthropic, xAI/Grok, Google, local Ollama)
- Support provider auth and account modes via config (API keys first; subscription-compatible flows when officially available)
- Streaming responses and async execution
- Retries, rate-limit handling, and graceful fallback across providers

Storage and Persistence

- Storage backend is abstracted behind a trait (swappable)
- SQLite is the first implementation (migrations + schema versioning)
- Persist sessions, messages, and agent/workflow state

Plugins

- Plugin system for tools early (dynamic loading), designed to keep the core clean
- Clear ABI/manifest expectations and compatibility story

Requirements Inputs

- `docs/initial-planning/REQUIREMENTS.md` defines capability and quality targets.
- `docs/initial-planning/tools.md` defines the target tool inventory and integration surface.
- These inputs guide roadmap prioritization and module boundaries.

Strategic Roadmap Guidance

- The phased implementation plan is the execution path; this document is the decision filter.
- When forced to choose: prefer clean boundaries, bounded resource usage, and extensibility over short-term hacks.

Vision Statement

Rustic-AI is a reliable, composable agentic engine that combines Rust's safety/performance with modern LLM tooling: multi-agent orchestration, durable sessions, efficient context management, and real-world tool execution (including interactive remote ops) in a clean, minimal, library-first design.
