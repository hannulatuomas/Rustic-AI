# TODO.md

This is the single source of truth for implementation tasks.

Last updated: 2026-02-10

## Current Focus
- End-to-end working agent session flow (in progress, needs debugging)

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
- [x] Add `config.example.json` and `docs/config.schema.json` for explicit setup and validation
- [x] Add config mutation foundation (`ConfigManager`, typed `ConfigPath`, atomic partial updates)
- [x] Add CLI config operations (`snapshot/get/set/unset/patch`) with scope-aware writes and effective-value source visibility
- [x] Add machine-readable config CLI responses (`--output json`) for frontend/API adapter consumption
- [x] Add stable JSON envelope contract for config CLI output (`rustic-ai-cli/config-output/v1`)
- [x] Add stable error envelope for config CLI JSON output (`status: error`, `code`, `message`, `details`)
- [x] Add example payloads for config CLI JSON envelopes (`docs/examples/config-cli-output.json`)
- [x] Add explicit `config.snapshot` JSON envelope example payload

## In Progress (Phase 1+ - Foundation for Interactive Sessions)
- [ ] Fix compilation errors in tool/agent permission/event system
- [ ] Complete ShellTool with proper tokio::process feature handling
- [ ] Implement ToolManager with proper permission resolution
- [ ] Implement Agent act loop with proper SessionManager integration
- [ ] Wire interactive loop in CLI with proper event handling
- [ ] Test end-to-end session flow with OpenAI provider

## Next (Phase 1+)
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
- [x] Add `config.example.json` and `docs/config.schema.json` for explicit setup and validation
- [x] Add config mutation foundation (`ConfigManager`, typed `ConfigPath`, atomic partial updates)
- [x] Add CLI config operations (`snapshot/get/set/unset/patch`) with scope-aware writes and effective-value source visibility
- [x] Add machine-readable config CLI responses (`--output json`) for frontend/API adapter consumption
- [x] Add stable JSON envelope contract for config CLI output (`rustic-ai-cli/config-output/v1`)
- [x] Add stable error envelope for config CLI JSON output (`status: error`, `code`, `message`, `details`)
- [x] Add example payloads for config CLI JSON envelopes (`docs/examples/config-cli-output.json`)
- [x] Add explicit `config.snapshot` JSON envelope example payload

## Phase Backlog (High Level)

### Phase 2 - Storage
- [x] Finalize `StorageBackend` trait surface
- [x] Complete SQLite implementation with migrations and indexes
- [ ] Add storage integration tests (including in-memory DB; deferred until full working foundation)

### Phase 3 - Providers
- [ ] Finalize provider trait shape (streaming + token helpers)
- [ ] Implement Anthropic/Grok/Google/Ollama providers
- [x] Implement OpenAI provider baseline
- [x] Add provider registry auto-wiring from config
- [x] Make provider schema type-specific (open_ai strict requirements; other providers extensible via `settings`)

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
- [ ] Config hot-reload: file watcher -> `ConfigManager.reload()` -> `ConfigChanged` event bus publish (debounced/coalesced)
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
