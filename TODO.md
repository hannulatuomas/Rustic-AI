# TODO.md

Single source of truth for active implementation work.

Last updated: 2026-02-10

## Current Focus
- Wire agent turn loop to handle permission-approved follow-up

## In Progress
- [ ] Wire agent turn loop to handle permission-approved follow-up
  - [x] Add `PendingToolState` model to storage
  - [x] Add `pending_tool_timeout_secs` to `PermissionConfig` schema (default: 300s)
  - [x] Add `StorageBackend` trait methods for pending tool operations
  - [x] Implement SQLite pending tool storage with schema migration (v2)
  - [x] Add `SessionManager` methods for pending tool operations
  - [x] Modify `Agent::run_assistant_tool_loop` to store pending state on None
  - [x] Add `Agent::resume_from_pending_tool` resume method
  - [x] Modify `Agent::continue_after_tool` to use resume path
  - [x] Add `RusticAI` cleanup for stale pending tools on startup

- [ ] Implement Filesystem tool
  - [x] Implement core filesystem operations (`read`, `write`, `edit`, `list`, `mkdir`, `delete`, `copy`, `move`, `info`, `glob`, `hash`)
  - [x] Wire `filesystem` tool registration in `ToolManager`
  - [x] Add runtime tool execution context plumbing (working directory) and apply it to shell/filesystem
  - [x] Extend permission model foundation to path-aware and agent-scoped allow/deny checks
  - [x] Add explicit permission prompts/resolutions integration for path-outside-root "ask" flows in active agent loop
- [ ] Implement HTTP tool
  - [x] Implement HTTP tool request execution (`GET`/`POST`/`PUT`/`PATCH`/`DELETE` and generic methods)
  - [x] Add bounded response streaming and timeout handling
  - [x] Wire `http` tool registration in `ToolManager`
  - [x] Integrate HTTP tool invocation into active agent tool-calling loop (JSON tool call protocol)

- [ ] Agent tool-call orchestration improvements
  - [x] Parse JSON tool-call responses and execute via ToolManager
  - [x] Feed tool outputs back into follow-up model response generation
  - [x] Implement interactive permission resolution flow in REPL (`PermissionRequest` -> user decision -> `resolve_permission`)
  - [x] Trigger assistant follow-up completion after permission-approved delayed tool execution
  - [x] Support multi-round autonomous tool loops with configurable per-agent limits
  - [x] Add per-agent total tool-call cap per turn (`max_total_tool_calls_per_turn`)
  - [x] Add per-agent turn duration cap (`max_turn_duration_seconds`) with unlimited option (`0`)
  - [ ] Add provider-focused integration tests (auth modes, non-stream and stream responses)

- [ ] Implement skills + workflows foundation (n8n-oriented)
  - [x] Add config schema for skills/workflows (including script execution mode and recursion limits)
  - [x] Implement skill discovery/registry with instruction + script skill loading
  - [x] Implement workflow discovery/registry with JSON+YAML loading and entrypoint/step validation
  - [x] Add CLI inspection commands (`/skills list|show`, `/workflows list|show`)
  - [ ] Integrate workflow execution engine with named outputs and downstream step routing
  - [ ] Add agent invocation support for skills/workflows

- [ ] Permission ergonomics and policy scopes
  - [x] Add config-level global/project allowed path lists
  - [x] Add config-level global/project shell command pattern policies (`allow`/`ask`/`deny`)
  - [x] Add runtime session/project/global permission overrides via REPL `/perm` commands
  - [x] Persist runtime `/perm global|project` additions back into config scopes automatically

- [x] Harden shell sudo execution flow
  - [x] Add permission/tool config fields for sudo TTL + privileged command matching
  - [x] Add `SudoSecretPrompt` event and renderer support
  - [x] Detect privileged shell commands and emit sudo prompt event
  - [x] Wire secure secret input + sudo command resume path (no persistence)

- [x] Implement SSH persistent session tool
  - [x] Add `ssh` tool registration in `ToolManager`
  - [x] Implement `connect`/`exec`/`disconnect` operations with streamed output
  - [x] Reuse existing SSH control session across multiple commands
  - [x] Add `list_sessions` and `close_all` operations
  - [x] Add PTY mode (`exec` with `pty=true`) and SCP upload/download operations

- [x] Implement MCP adapter integration
  - [x] Add MCP config schema (`mcp.servers`) with validation
  - [x] Register `mcp` tool behind `features.mcp_enabled`
  - [x] Implement MCP stdio adapter operations (`list_servers`, `list_tools`, `call_tool`)
  - [x] Add config schema/example documentation updates

- [x] Implement plugin tool loader wiring
  - [x] Add plugins config schema (`plugins.directories`, `plugins.manifest_file_name`, `plugins.max_discovery_depth`)
  - [x] Implement manifest discovery + validation for plugin command tools
  - [x] Load and register plugin tools in `ToolManager` behind `features.plugins_enabled`

- [x] Config ergonomics
  - [x] Support layered config fragments for global and project scopes
  - [x] Add explicit docs/examples for recommended split-file layouts (`agents.json`, `tools.json`, `providers.json`, `permissions.json`)

## Next
- [ ] Add provider-focused integration tests (auth modes, non-stream and stream responses)

## Done
- [x] Initialize workspace (`rustic-ai-core`, `frontend/rustic-ai-cli`)
- [x] Build/lint/test baseline green for initial foundation
- [x] Implement core config loading, merge, and validation framework
- [x] Implement config mutation layer (`ConfigManager`, typed paths, atomic writes)
- [x] Implement storage abstraction and SQLite backend with session/message persistence
- [x] Implement provider registry and factory boundary
- [x] Implement OpenAI provider baseline (`generate`)
- [x] Extend provider generation model (`GenerateOptions`, `ChatMessage`, function-call metadata)
- [x] Extend `ModelProvider` trait with `stream_generate`, `count_tokens`, capability flags
- [x] Add provider SSE parsing utility module (`providers/streaming.rs`)
- [x] Upgrade OpenAI provider:
  - [x] robust request construction (advanced options + typed payload)
  - [x] streaming response handling via SSE parser
  - [x] `auth_mode: subscription` support with explicit header/config handling
  - [x] remove panic-based header/client construction in runtime path
- [x] Implement subscription auth subsystem:
  - [x] browser OAuth login flow (local callback receiver + PKCE)
  - [x] headless/device-code flow
  - [x] credential persistence in local auth store (`~/.rustic-ai/data/auth.json` by default)
  - [x] token refresh handling for runtime provider requests
  - [x] CLI auth commands (`auth connect`, `auth list`, `auth logout`)
- [x] Integrate OpenAI provider subscription auth with persisted credentials (API key no longer required for subscription mode)
- [x] Implement Anthropic provider (generate + stream + token count) and wire into provider factory
- [x] Implement Anthropic authentication parity:
  - [x] `auth_mode: api_key`
  - [x] `auth_mode: subscription` via shared browser/headless auth subsystem
  - [x] runtime token refresh integration for subscription calls
- [x] Implement Google provider (generate + stream + token count) and wire into provider factory
- [x] Implement Google authentication parity:
  - [x] `auth_mode: api_key`
  - [x] `auth_mode: subscription` via shared browser/headless auth subsystem
  - [x] runtime token refresh integration for subscription calls
- [x] Implement Grok provider (generate + stream + token count) with API-key-only auth
- [x] Implement Z.ai provider with dual endpoint support (general and coding) and API-key auth
- [x] Implement custom OpenAI-compatible provider (endpoint + api key)
- [x] Implement Ollama provider (generate + stream + token count)
- [x] Add provider auth capability visibility and enforcement:
  - [x] central provider auth-mode capability mapping
  - [x] validation errors list supported auth modes per provider type
  - [x] CLI command `auth methods` to show configured vs supported auth modes
  - [x] default `api_key` path available across configured provider types
  - [x] `subscription` limited to open_ai, anthropic, google
- [x] Update OpenAI factory wiring for auth-mode aware construction and settings parsing
- [x] Update config validation/schema for `auth_mode: subscription`
- [x] Implement tool system foundation (`Tool` trait, registry, manager)
- [x] Implement Shell tool baseline with permission checks and streaming events
- [x] Implement permission policy foundation (`allow` / `deny` / `ask`)
- [x] Implement agent/session flow foundation and CLI interactive loop baseline

## Verification Commands
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt --all`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --workspace`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --workspace --all-features`

## Update Rules
- Every non-trivial change updates this file in the same change.
- Keep only one active tracker (this file).
- Move completed work to Done immediately.
- Reflect scope changes before implementation proceeds.
