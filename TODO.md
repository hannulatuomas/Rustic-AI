# TODO.md

Single source of truth for active implementation work.

Last updated: 2026-02-12

## Current Status Snapshot

- Requirements coverage is strong in core runtime capabilities (providers, workflows, permissions, learning, indexing/RAG).
- Main near-term work is platform fit/finish, domain breadth, and context-efficiency improvements.
- Tests/CI are intentionally deferred for now (explicit product decision).

## Active Priorities (Now)

### 1) Context-Efficient Agent Prompting

- [ ] Replace static agent tool list prompt injection with context-aware shortlist generation.
  - [x] Use `ToolManager::get_tool_descriptions(...)` in agent turn prompt construction.
  - [x] Add prompt budget guardrails for tool descriptions (max items + max chars/tokens).
  - [x] Prefer task-relevant tools first (keyword/focus-aware ranking).
  - [ ] Add config toggles for shortlist mode (`full`, `priority`, `task_focused`).

- [ ] Improve sub-agent discovery context without bloating prompts.
  - [ ] Add optional target-agent shortlist helper (capability + taxonomy filtered).
  - [ ] Avoid emitting full agent catalogs in sub-agent invocation guidance.
  - [ ] Add hard limits for sub-agent metadata injected into prompts.

### 2) Tool and Agent Breadth for Target Use Cases

- [ ] Add missing high-impact tool coverage from requirements/tools inventory.
  - [ ] `web_fetch` tool (deterministic fetch-focused variant).
  - [ ] `crawler` tool (link extraction + robots-aware bounds).
  - [ ] `process` tool (background process start/status/stop with safety constraints).
  - [ ] `docker` tool (local Docker/Podman baseline operations with permission gates).

- [ ] Add practical domain agent presets and routing keywords.
  - [ ] DevOps/container agent preset.
  - [ ] Security assessment/pentest assistant preset.
  - [ ] Windows administration preset.
  - [ ] Data/ML operations preset.
  - [ ] Azure/Microsoft ecosystem preset.

### 3) Taxonomy and Catalog Usability

- [ ] Ship non-empty default taxonomy baskets/sub-baskets in example config.
  - [ ] Development, DevOps, Security, Data/ML, Cloud, Infra, Docs.
  - [ ] Map built-in tools and sample agents to baskets.
  - [ ] Add quick CLI examples for taxonomy-driven discovery.

- [ ] Improve registry discovery ergonomics.
  - [ ] Add combined filter command for agents by tool + basket + permission mode.
  - [ ] Add ranked suggestions in CLI for `agent suggest` task intent flow.

### 4) Routing and Orchestration Consistency

- [ ] Apply dynamic routing consistently across execution entrypoints.
  - [ ] Ensure behavior parity between REPL and non-REPL command paths.
  - [ ] Integrate routing hooks for workflow-triggered agent execution paths.

- [ ] Tighten routing trace + TODO linkage.
  - [ ] Add richer trace metadata to TODO auto-created tasks.
  - [ ] Add CLI view to inspect route->todo->sub-agent chain in one output.

### 5) Docs and Repository Hygiene

- [ ] Keep only active/reference docs that match implemented behavior.
- [ ] Remove completed planning docs that no longer drive execution.
- [ ] Keep README/TODO/DECISIONS synchronized when scope changes.

### 6) Dependency Compatibility Hardening

- [x] Resolve `sqlx-postgres v0.8.0` future-incompat warning.
  - [x] Unblock upgrade path beyond `sqlx 0.8.0` by upgrading `tree-sitter-*` dependency stack.
  - [x] Upgrade sqlx stack to patched version (`0.8.6`).

### 7) Runtime Quality Hardening

- [ ] Continue replacing sync filesystem access in async execution paths outside current hotspots.
  - [x] Prioritize rule/config loading paths that may run during active sessions.
  - [x] Prioritize tool paths used in long-running operations.

### 8) Comprehensive Backend Refactor Plan (Core-only)

- [x] Phase A: Reduce `workflows/executor.rs` complexity with per-step handlers and shared policy/retry utilities.
  - [x] Extract `Tool` and `Skill` handlers.
  - [x] Extract `Switch` and `Agent` handlers.
  - [x] Extract `Condition` and `Wait` handlers.
  - [x] Extract `Workflow` nested-call handler and unify step-finalization paths.
- [x] Phase B: Reduce `agents/behavior.rs` loop complexity with explicit turn state helpers.
  - [x] Unify turn-duration budgeting.
  - [x] Extract round orchestration and tool-call execution subroutines.
  - [x] Standardize tool-result serialization/error mapping helpers.
- [ ] Phase C: Tighten runtime invariants and deterministic behavior.
  - [x] Deterministic discovery ordering (skills/workflows/plugins).
  - [ ] Add focused tests for extracted executor branch handlers (deferred).
  - [ ] Add focused tests for agent turn interruption/timeout behavior (deferred).
  - [ ] Add focused tests for switch resolution priority/validation and tool output message rendering (deferred).

## Deferred (By Decision)

### Reliability Gates (Deferred for now)

- [x] Remove current core tests to allow full test-plan redesign before reimplementation.
- [ ] Comprehensive tests expansion (unit/integration/e2e/perf).
- [ ] CI pipeline and mandatory quality gates.

### Productization (Deferred for now)

- [ ] API server runtime path (REST + streaming + auth + multi-user).
- [ ] TUI frontend.

## Recently Completed Highlights

- [x] Agent read-only vs read-write permission mode with tool-level enforcement.
- [x] Sub-agent protocol with context filtering, depth controls, and resume flow.
- [x] Sub-agent orchestration v2 (parallel execution, caching, visibility, TODO integration).
- [x] Dynamic routing (hybrid policy, fallback agent, trace persistence, CLI integration).
- [x] Taxonomy registry + CLI discovery/search commands.
- [x] Advanced context management (importance scoring, dedupe, summarization controls).
- [x] Big-codebase support (indexing, vector search, hybrid retrieval, graph/impact analysis).
- [x] Learning subsystem (feedback, patterns, preferences, success patterns).
- [x] Core panic hardening (`unwrap`/`expect` removed from runtime and tests).
- [x] Async file I/O hardening in indexing/RAG/session rule loading/LSP document open paths.
- [x] Reduced discovery/path traversal overhead by switching directory scans to `DirEntry::file_type()` in workflows/skills/plugins/rules/config fragments.
- [x] Defined sync-vs-async loader policy (startup loaders stay sync; runtime paths prioritized for async hardening).
- [x] Deterministic loader discovery ordering for skills/workflows/plugins (stable file sorting).
- [x] Git diff output handling hardened for non-UTF8 hunks (`from_utf8_lossy`).
- [x] Tool maintainability cleanup: centralized git command string mapping and LSP server-args parsing helper.
- [x] Large-module maintainability refactors: extracted workflow resolution/recursion helpers in executor and unified agent turn-duration budget handling.
- [x] Executor retry-branch deduplication via shared backoff helper/context and standardized route-as-failure payloads.
- [x] Extended executor policy deduplication for switch/agent retry flows via shared helpers.
- [x] Extracted dedicated tool/skill step execution helpers from executor main loop.
- [x] Extracted dedicated switch/agent step execution helpers from executor main loop.
- [x] Extracted dedicated condition/wait/workflow step execution helpers and centralized workflow completion finalization.
- [x] Agent behavior refactor: centralized turn-budgeted tool execution helper/context in autonomous loop.
- [x] Batch refactor pass: reduced executor boilerplate with shared step context/counter factories and moved disallowed-tool handling into a single agent helper.
- [x] Batch refactor pass: centralized step completion/next-step routing helpers and improved workflow step error context for skill/agent resolution failures.
- [x] Batch refactor pass: centralized condition/switch post-processing via dedicated branch helpers while preserving early-exit semantics.
- [x] Batch refactor pass: extracted shared step runtime/finalization helpers to reduce `run_internal` branching boilerplate without behavior changes.
- [x] Batch refactor pass: extracted standard step post-processing helper (outputs + completion event + next-step routing) to reduce duplicated run-loop logic.
- [x] Batch refactor pass: unified early-exit advance/finalize control path via shared target-advance helper.
- [x] Batch refactor pass: improved executor structure with shared step resolution/timeout/start-event helpers and unified tool-call budget enforcement in agent loops.
- [x] Batch refactor pass: moved shared step runtime/counter setup outside per-branch match arms to further reduce executor control-flow duplication.

## Verification Commands

- `export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt --all`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --workspace`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Update Rules

- Every non-trivial change updates this file in the same change.
- Keep this file as the single active tracker.
- Move completed work to "Recently Completed Highlights" promptly.
- Reflect scope decisions (including deferrals) before continuing implementation.

## Reference Documents

- Requirements: `docs/initial-planning/REQUIREMENTS.md`
- Tool Inventory: `docs/initial-planning/tools.md`
- Design Guide: `docs/DESIGN_GUIDE.md`
- Decisions: `docs/DECISIONS.md`
- State Analysis: `docs/comprehensive-state-analysis.md`
- Big Picture: `docs/initial-planning/big-picture.md`
- Integration Plan: `docs/initial-planning/integration-plan.md`
