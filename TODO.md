# TODO.md

This is the single source of truth for implementation tasks.

Last updated: 2026-02-09

## Current Focus
- Phase 1 foundation work (config + runtime wiring + core API)

## Done
- [x] Initialize workspace (`rustic-ai-core`, `frontend/rustic-ai-cli`)
- [x] Build/lint/test baseline is green
- [x] Implement base error type + `Result<T>`
- [x] Add config schema with typed enums and defaults
- [x] Implement config loading from file
- [x] Implement config loading from env
- [x] Implement config merge strategy
- [x] Implement config validation (required sections + cross-reference checks)
- [x] Add event bus foundation (`EventBus`)
- [x] Add runtime container (`Runtime`)
- [x] Add `RusticAI` constructor and config-path constructor
- [x] Wire CLI startup to load config and initialize `RusticAI`
- [x] Add basic registry accessors (`get`, `list`, `is_empty`)
- [x] Add initial unit tests for config loader/validation

## In Progress
- [ ] Phase 1 completion pass (finish remaining non-deferred tasks)

## Next (Phase 1)
- [x] Config: load `.cursorrules` and `.windsurfrules` into rule sources
- [x] Config: load context files (`AGENTS.md`, others) with deterministic precedence
- [x] Config: implement explicit rule precedence (`global -> project -> topic/session -> runtime`)
- [x] Storage: complete backend abstraction methods needed for sessions/history
- [x] Storage: implement SQLite schema + CRUD (first minimal usable slice)
- [x] Storage manager: add high-level operations used by session manager
- [x] Conversation: implement session manager methods (create/get/list/delete)
- [x] Conversation: implement context window manager behavior
- [x] Runtime wiring: connect storage + conversation into `RusticAI`
- [x] CLI: add minimal command behavior beyond initialization (single-run path)
- [x] Session manager: consume discovered `rules.discovered_rules` metadata and load rule contents on demand
- [x] Agent/session flow: wire LLM-based topic inference into `TopicTracker` to switch context-specific rules efficiently
- [x] Rule frontmatter: support Cursor-like metadata (`description`, `globs`, `alwaysApply`) with manual invocation support (`@rule-name`)
- [x] Config format: use JSON (`config.json`) as default load format
- [x] Storage paths: split global settings/data (`~/.rustic-ai`) from project session data (`<workdir>/.rustic-ai`), all configurable
- [x] Remove hardcoded runtime/provider/storage fallback values and require explicit provider/storage config inputs

## Phase Backlog (High Level)

### Phase 2 - Storage
- [ ] Finalize `StorageBackend` trait surface
- [ ] Complete SQLite implementation with migrations and indexes
- [ ] Add storage integration tests (including in-memory DB)

### Phase 3 - Providers
- [ ] Finalize provider trait shape (streaming + token helpers)
- [ ] Implement Anthropic/Grok/Google/Ollama providers
- [x] Implement OpenAI provider baseline
- [ ] Add provider registry auto-wiring from config

### Phase 4 - Tools/Skills/Plugins
- [ ] Finalize tool trait and metadata schema
- [ ] Implement shell/ssh/filesystem/http tools (safe defaults)
- [ ] Implement plugin loader and MCP adapter wiring
- [ ] SSH tool: support both key-based and username/password auth, prompt credentials per use, never persist or log credentials

### Phase 5-9 - Agents/Workflows/Conversation/CLI
- [ ] Implement agent core + memory/state
- [ ] Implement coordinator and multi-agent orchestration
- [ ] Implement workflow parser/executor and command routing
- [ ] Implement robust session/history APIs
- [ ] Expand CLI into full interactive + batch workflow support

### Phase 10 - Reliability, Policy, and Runtime Hardening
- [ ] Permission policy enforcement end-to-end
- [ ] Reliability patterns (retry/fallback/circuit breaker/shutdown)
- [ ] Logging/tracing polish (`init_logging`, prod json + dev pretty output, trace correlation)
- [ ] Performance profiling and optimization
- [ ] Hardening/extensibility checks

### Phase 11 - Documentation and Examples
- [ ] Public API documentation/examples
- [ ] Error variant and configuration reference docs
- [ ] User/developer guides and runnable examples

## Verification Commands
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt --all`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --workspace`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --workspace --all-features`

## Update Rules (Mandatory)
- Every non-trivial change must update this file in the same change.
- Move finished items to Done immediately.
- If scope changes, reflect it here before implementation continues.
- Do not keep parallel task trackers in other docs as the active source.
