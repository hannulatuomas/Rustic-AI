# Rustic-AI: Comprehensive State Analysis

Date: 2026-02-11
Scope: Current repository state (`main` + local in-progress changes)
Method: Direct code inspection + delegated sub-agent reviews (codebase analysis, code quality review, market comparison)

---

## Executive Summary

Rustic-AI is now a substantial, library-first Rust agent runtime with strong breadth in providers, tools, workflow orchestration, policy controls, learning, and large-codebase retrieval.

At a high level, the project is best described as:
- a policy-aware coding/automation engine (core crate)
- with a practical CLI frontend
- with explicit workflow semantics (n8n-inspired)
- and recently completed indexing/vector/RAG capabilities

Overall posture:
- Architecture and module boundaries: strong
- Feature implementation depth: strong in core runtime paths
- Operational maturity (tests/CI/release hardening): still a key gap

---

## 1) What The Program Can Do Right Now

### 1.1 Core runtime and UX

The system can:
- Run interactive sessions via CLI REPL (`chat`) with event streaming and permission prompts.
- Manage sessions persistently (create/list/continue/delete).
- Execute agent turns with tool-call loops, pending tool resume, sub-agent calls, and interruption.
- Emit rich events for model chunks, tool lifecycle, workflow lifecycle, permissions, learning, and retrieval context injection.

Key files:
- `rustic-ai-core/src/lib.rs`
- `rustic-ai-core/src/runtime/mod.rs`
- `rustic-ai-core/src/agents/behavior.rs`
- `rustic-ai-core/src/events/types.rs`
- `frontend/rustic-ai-cli/src/repl.rs`
- `frontend/rustic-ai-cli/src/renderer.rs`

### 1.2 Provider support (LLM)

Implemented providers:
- OpenAI
- Anthropic
- Google
- Grok
- Z.ai
- Ollama
- Custom OpenAI-compatible

Capabilities include:
- Unified provider trait
- Streaming support
- Retry handling
- subscription auth flows (where supported)

Key files:
- `rustic-ai-core/src/providers/`
- `rustic-ai-core/src/providers/factory.rs`
- `rustic-ai-core/src/providers/types.rs`

### 1.3 Tool system

Implemented tools and adapters include:
- shell, filesystem, http, ssh
- git, grep, database, web_search, download
- regex, format, encoding, convert, image, lsp
- mcp, skill, sub_agent

Tool execution supports:
- permission mediation (allow/deny/ask)
- read-only/read-write agent modes
- streaming output and pending-resolution flows

Key files:
- `rustic-ai-core/src/tools/mod.rs`
- `rustic-ai-core/src/tools/manager.rs`
- `rustic-ai-core/src/tools/*.rs`

### 1.4 Workflow engine (n8n-inspired semantics)

Implemented workflow capabilities include:
- step kinds: tool, skill, agent, workflow, condition, wait, loop, merge, switch
- grouped conditions, expression parsing/evaluation, retry/timeout controls
- routing via success/failure branches
- trigger metadata and trigger engine structures

Key files:
- `rustic-ai-core/src/workflows/types.rs`
- `rustic-ai-core/src/workflows/executor.rs`
- `rustic-ai-core/src/workflows/expressions.rs`
- `rustic-ai-core/src/workflows/trigger.rs`

### 1.5 Permissions and safety

Implemented:
- allow/deny/ask policy
- persistent session decisions
- command/path pattern controls
- read-only vs read-write enforcement by agent mode

Key files:
- `rustic-ai-core/src/permissions/configurable_policy.rs`
- `rustic-ai-core/src/permissions/policy.rs`
- `rustic-ai-core/src/config/schema.rs`

### 1.6 Learning subsystem (Phase 6)

Implemented:
- explicit feedback capture (CLI `/feedback`)
- implicit feedback from events/errors/tool outcomes
- mistake pattern tracking and warnings
- preference tracking/application
- success pattern storage and retrieval

Key files:
- `rustic-ai-core/src/learning/mod.rs`
- `rustic-ai-core/src/learning/types.rs`
- `rustic-ai-core/src/learning/*.rs`

### 1.7 Big-codebase support (Phase 7)

Implemented:
- code indexing with symbol extraction and call edges
- persistent index storage (SQLite/Postgres schema support)
- vector persistence and cosine search
- hybrid retrieval (keyword + vector)
- context expansion, ranking adjustments, token-budgeted prompt injection
- graph and impact analysis + DOT export
- CLI diagnostics (`index status`, `index retrieve`, `index graph`, `index impact`)

Key files:
- `rustic-ai-core/src/indexing/mod.rs`
- `rustic-ai-core/src/indexing/ast.rs`
- `rustic-ai-core/src/indexing/graph.rs`
- `rustic-ai-core/src/vector/mod.rs`
- `rustic-ai-core/src/rag/mod.rs`
- `frontend/rustic-ai-cli/src/cli.rs`
- `frontend/rustic-ai-cli/src/main.rs`

### 1.8 Configurability

Implemented toggle/config surfaces include:
- feature toggles (`learning`, `indexing`, `vector`, `rag`, etc.)
- retrieval knobs (top-k, score thresholds, ranking weights, token budget, injection mode)
- embedding backend selection (`deterministic_hash`, `open_ai`, `open_ai_compatible`, `sentence_transformers`)
- optional SQLite vector extension loading with strict/best-effort mode

Key files:
- `rustic-ai-core/src/config/schema.rs`
- `rustic-ai-core/src/config/loader.rs`
- `rustic-ai-core/src/config/validation.rs`
- `docs/config.schema.json`

---

## 2) Comparison: Rustic-AI vs OpenCode, Claude Code, n8n

Note: External comparisons are positioning-level and should be treated as directional.

### 2.1 Against OpenCode and Claude Code

Rustic-AI is currently stronger in:
- explicit policy programmability (allow/deny/ask + scoped patterns)
- explicit workflow engine semantics (beyond chat-loop tasking)
- pluggable architecture across providers/tools/storage/retrieval

Rustic-AI is currently weaker in:
- end-user polish and developer ergonomics out of the box
- confidence signals from automated testing and CI visibility
- likely production packaging maturity (service/API-centric deployment story still pending)

Positioning:
- OpenCode/Claude Code: optimized coding-assistant UX first
- Rustic-AI: customizable orchestration runtime with coding + automation + policy emphasis

### 2.2 Against n8n

Rustic-AI is currently stronger in:
- coding-agent-native tool loops and model orchestration
- agent/sub-agent patterns and permission-centric controls

Rustic-AI is currently weaker in:
- visual workflow UX
- breadth of SaaS connector ecosystem
- workflow marketplace/community maturity

Positioning:
- n8n: integration/workflow automation platform first
- Rustic-AI: AI agent runtime that can express workflow-like control flow in code/config

### 2.3 Practical takeaway

Rustic-AI differentiates best as:
- self-hostable
- policy-aware
- deeply configurable
- coding/ops friendly

The biggest competitive limiter is not core capability breadth anymore; it is operational trust and productization maturity.

---

## 3) Code Quality and Consistency

### 3.1 Strengths

- Strong crate boundary discipline: core logic in `rustic-ai-core`, UX in `frontend/rustic-ai-cli`.
- Good use of traits/interfaces for major seams (`ModelProvider`, `Tool`, `StorageBackend`, embedding provider abstraction).
- Consistent typed config and validation pipeline.
- Migration-versioned persistence evolution with coordinated schema updates.
- Event model is broad and integrated across subsystems.

### 3.2 Notable debt/risk areas

- Test coverage remains the major risk (still low relative to feature breadth).
- No clearly visible CI pipeline in repo for mandatory quality gates.
- Some modules are becoming large/complex (`agents/behavior.rs`, large workflow/config paths).
- Provider/tool logic still contains some repetition opportunities.

### 3.3 Security/safety posture snapshot

- Positive: policy model and permission workflow are explicit and strong for local-agent workflows.
- Watchlist: maintain strict handling for command/path filtering edge cases and credential material in memory/logging.

---

## 4) Integration and Fit

### 4.1 How components fit together

High-level flow:
- CLI command or REPL input
- `RusticAI` facade boots config + storage + runtime components
- Agent/workflow execution uses provider registry + tool manager + permission policy
- Session and state persist via storage backend
- Retrieval and learning modules enrich execution context and adaptation
- Event bus streams to renderer for UX and observability

This composition is coherent and mostly consistent with the architecture goals in docs.

### 4.2 Integration quality assessment

- Core integrations are now substantially complete for learning and RAG/indexing.
- Storage migrations track the feature expansion coherently.
- CLI now exposes meaningful diagnostics for indexing/retrieval state.
- The main remaining integration concerns are around hardening and regression safety rather than missing wiring.

---

## 5) Current State vs TODO

From repository state and TODO:
- Advanced Tools: implemented and now reflected as complete.
- Code Graph Analysis: implemented and reflected as complete.
- Big-codebase support (indexing/vector/RAG/integration): implemented and reflected as complete.

Main remaining high-priority program-level work is now concentrated in:
- tests and CI/reliability gates
- release/productization artifacts (README alignment, API server/TUI/release hardening)

---

## 6) Recommended Next Priorities

1. Reliability gate first:
- Add systematic unit/integration coverage for high-risk modules (agents, workflows, permissions, retrieval, storage migrations).
- Add CI pipeline enforcing fmt/build/clippy/tests.

2. Operational packaging:
- Define and implement service/API runtime path for non-CLI operation.
- Add production-oriented deployment/runbook docs.

3. Quality-focused refactors:
- Reduce complexity in large agent/workflow/config modules.
- Extract duplicated provider/tool patterns where practical.

4. Documentation synchronization:
- Keep this analysis and README consistently aligned with implemented reality.

---

## 7) Bottom Line

Rustic-AI is no longer just foundational scaffolding. It is now a capable agent runtime with broad local tooling, strong control-flow semantics, and meaningful large-codebase retrieval features.

Compared to OpenCode/Claude Code/n8n, the primary gap is no longer feature intent; it is confidence and maturity signals (tests/CI/product hardening) and end-user polish.

If reliability and release hardening are prioritized next, the project has a credible path to a robust v1-grade platform.
