# TODO.md

Single source of truth for active implementation work.

Last updated: 2026-02-12

## Current Status Snapshot

- Core runtime is in good shape for providers, tools, workflows, permissions, learning, and indexing/RAG.
- Prompt efficiency, taxonomy usability, and routing consistency workstreams are complete for current scope.
- Test/CI redesign remains intentionally deferred by product decision.

## Active Priorities

### 1) Tool and Agent Breadth

- [x] Add missing high-impact tool coverage from requirements/tools inventory.
  - [x] `web_fetch`
  - [x] `crawler`
  - [x] `process`
  - [x] `docker`
  - [x] `code_search`
  - [x] `watch`

- [x] Add practical domain agent presets and routing keywords.
  - [x] Programming languages + scripting preset.
  - [x] Frameworks/web stack preset.
  - [x] Linux administration preset.
  - [x] Database/data analysis preset.
  - [x] API development/maintenance preset.
  - [x] Cyber security/pentest/malware/OSINT preset.
  - [x] Microsoft/Azure preset.
  - [x] DevOps/container agent preset.
  - [x] DevSecOps preset.
  - [x] Security assessment/pentest assistant preset.
  - [x] Windows administration preset.
  - [x] Game development preset.
  - [x] IaC preset.
  - [x] Cloud/servers/VM/networking preset.
  - [x] Data/ML operations preset.
  - [x] AI/ML + prompt engineering preset.
  - [x] Azure/Microsoft ecosystem preset.

### 2) Documentation and Repo Hygiene

- [ ] Keep only active/reference docs that match implemented behavior.
- [ ] Remove completed planning docs that no longer drive execution.
- [ ] Keep `README.md`, `TODO.md`, and `docs/DECISIONS.md` synchronized when scope changes.

### 3) Runtime Quality Hardening

- [x] Continue replacing sync filesystem access in async execution paths outside current hotspots.
  - [x] Rule/config loading paths used during active sessions.
  - [x] Long-running tool paths.
  - [x] Offload heavy filesystem and grep scans via blocking task pool.
  - [x] Remove synchronous workspace directory scan from async sub-agent context path.

### 4) Comprehensive Backend Refactor Plan (Core-only)

- [x] Phase A: Reduce `workflows/executor.rs` complexity with per-step handlers and shared policy/retry utilities.
- [x] Phase B: Reduce `agents/behavior.rs` loop complexity with explicit turn-state helpers.
- [ ] Phase C: Tighten runtime invariants and deterministic behavior.
  - [x] Deterministic discovery ordering (skills/workflows/plugins).
  - [ ] Focused tests for extracted executor branch handlers (deferred).
  - [ ] Focused tests for agent interruption/timeout behavior (deferred).
  - [ ] Focused tests for switch resolution priority/validation and tool output rendering (deferred).

## Deferred (By Product Decision)

### Reliability Gates

- [x] Remove current core tests to allow full test-plan redesign before reimplementation.
- [ ] Comprehensive tests expansion (unit/integration/e2e/perf).
- [ ] CI pipeline and mandatory quality gates.

### Productization

- [ ] API server runtime path (REST + streaming + auth + multi-user).
- [ ] TUI frontend.

## Recently Completed Highlights

- [x] Context-efficient prompting: tool shortlist ranking/budgets and shortlist mode (`full`/`priority`/`task_focused`).
- [x] Sub-agent prompt slimming with bounded target-agent shortlist.
- [x] Taxonomy usability: non-empty default baskets, built-in mapping, and CLI taxonomy examples.
- [x] Registry ergonomics: `agents filter` (tool + basket + permission) and ranked `agents suggest`.
- [x] Routing consistency: shared routing trace/TODO path reused in REPL and workflow agent execution.
- [x] Routing chain observability: `routing chain` CLI output for route -> todo -> sub-agent linkage.
- [x] Richer routing TODO metadata (policy/confidence/fallback/alternatives context).
- [x] Added `code_search` and `watch` tools to close tool-inventory gaps.
- [x] Added domain-focused agent presets and expanded dynamic routing keywords in example config.
- [x] Expanded requirements coverage presets for programming languages, frameworks, Linux distros, databases, APIs, cyber security, Microsoft/Azure, Windows ops, DevOps/SecDevOps, AI/ML+prompt engineering, game dev, IaC, containers, cloud, servers/VMs/networking.
- [x] Async runtime hardening pass: moved grep/filesystem heavy sync operations off async reactor.
- [x] Runtime hardening/refactor sweep across executor/agent/tool paths with strict fmt/build/clippy passing.

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
