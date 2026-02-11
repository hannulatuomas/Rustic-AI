# TODO.md

Single source of truth for active implementation work.

Last updated: 2026-02-11

## Project Status

**Overall Requirements Coverage: ~60%**

**Current Implementation vs Requirements:**
- LLM Providers: 175% (7 implemented, 4 required)
- Tool System: 23% (7 implemented, 30+ required)
- Permissions: 50% (allow/deny/ask, missing read/write)
- Agent Coordination: 30% (partial, missing direct agent calling)
- Context Handling: 20% (simple LIFO, no summarization/RAG)
- Big Codebases: 0% (no indexing, vector DB, semantic search)
- Self-Learning: 75% (feedback, patterns, preferences, success library implemented)
- Taxonomy: 10% (schema only, no implementation)

**Estimated Completion: 22-26 weeks of work remaining**

---

## Implementation Roadmap

### Phase 1: Foundation - Agent Capabilities (Weeks 1-3)
**Priority: HIGH - Enables other features**

1. **Gap 4: Agent Registry Implementation** (2 days)
    - [x] Implement full AgentRegistry with query methods
    - [x] Add filtering by tool/skill/permission
    - [x] Add task suitability heuristics

2. **Gap 1: Agent Permission Model (Read-Only vs Read-Write)** (4-5 days)
    - [x] Extend PermissionPolicy with AgentPermissionMode
    - [x] Add PermissionAction::AllowRead/AllowWrite variants
    - [x] Add agent-level permission_mode config field
    - [x] Enforce read-only in Filesystem tool (read operations only)
    - [x] Enforce read-only in Shell tool (no write commands)
    - [x] Enforce read-only in SSH tool (read-only mode)
    - [x] Update ToolManager to pass permission mode to context
    - [x] Update CLI to show agent permission modes

3. **Gap 6: Agent-to-Agent Calling Protocol** (5 days)
    - [x] Create SubAgentTool with filtered context support
    - [x] Implement filtered context building (OpenCode-style)
    - [x] Implement workspace context summarization
    - [x] Add sub-agent execution in AgentCoordinator
    - [x] Add ContextFilter types (last_messages, include_roles, include_keywords, include_workspace)
    - [x] Add max_context_tokens control
    - [x] Register sub_agent tool in ToolManager
    - [x] Add allow_sub_agent_calls config to AgentConfig
    - [x] Update CLI to render sub-agent call events

### Phase 2: Organization & Context (Weeks 4-6)
**Priority: HIGH - Better organization and context usage**

4. **Gap 2: Taxonomy Implementation** (5-6 days)
    - [x] Extend AgentConfig with taxonomy_membership field
    - [x] Extend ToolConfig and skill structures with taxonomy
    - [x] Create TaxonomyRegistry module
    - [x] Implement Basket and SubBasket structures
    - [x] Implement filtering APIs (by_basket, by_sub_basket, search)
    - [x] Validate depth limit (max 2 levels)
    - [x] Wire taxonomy into initialization
    - [x] Update AgentRegistry to use taxonomy for filtering
    - [x] Update ToolRegistry to use taxonomy for filtering
    - [x] Add CLI commands: `/taxonomy list|show <basket> [sub_basket]|search <query>`

5. **Gap 7: Advanced Context Management** (8-10 days)
    - [x] Create MessageScorer module (importance levels)
    - [x] Implement importance scoring logic (critical/high/medium/low)
    - [x] Add context pruning with importance-based selection
    - [x] Create ContextDeduplicator module
    - [x] Implement duplicate detection and removal
    - [x] Implement provider-assisted summarization
    - [x] Add ConversationSummary storage structure
    - [x] Implement summary caching and expansion
    - [x] Implement dynamic context optimization per task
    - [x] Add keyword extraction from current task
    - [x] Implement task-relevance scoring
    - [x] Update AgentMemory to use new context building

6. **Gap 5: Dynamic Tool Loading** (3 days)
    - [x] Refactor ToolManager to support lazy loading
    - [x] Add lazy_loaders HashMap for heavy tools
    - [x] Implement lazy tool loading on demand
    - [x] Add active_tools tracking
    - [x] Implement unload_unused method
    - [x] Add get_tool_descriptions for agent tool lists
    - [x] Implement context-aware tool description selection
    - [x] Add tool priority ordering for truncation

### Phase 3: Code Quality & Refactoring (Weeks 7-8)
**Priority: HIGH - Technical debt and improvements**

7. **Address Code Duplication** (3-4 days)
    - [x] Extract common streaming logic from providers
    - [x] Create shared SSE streaming utility
    - [x] Extract provider option building logic
    - [x] Create provider option builder trait
    - [x] Simplify ToolManager::execute_tool()
    - [x] Extract helpers from ToolManager
    - [x] Reduce `Agent::resume_from_pending_tool()` complexity
    - [x] Extract pending tool state machine

8. **Complete Loose Integrations** (3-4 days)
    - [x] Wire workflow trigger execution
    - [x] Implement cron scheduling
    - [x] Implement event-based triggers
    - [x] Integrate topic inference into agent prompts
    - [x] Implement topic inference completion
    - [x] Implement project mode rule scoping
    - [x] Complete project mode profile loading
    - [x] Wire PostgreSQL backend
    - [x] Implement PostgreSQL storage backend
    - [x] Add custom storage backend support

9. **Plugin System & Examples** (4-5 days)
    - [ ] Add sample plugins to demonstrate loading framework
    - [ ] Document plugin development
    - [ ] Consider plugin sandboxing/security
    - [ ] Create plugin examples directory
    - [ ] Add plugin manifest examples
    - [ ] Write plugin development guide

### Phase 4: Workflow Engine Enhancements (Weeks 8-12)
**Priority: HIGH - N8n-like workflow capabilities**

10. **Phase 1: Grouped Conditions** (2-3 days)
    - [x] Add `LogicalOperator`, `Condition`, `ConditionGroup` types to `types.rs`
    - [x] Implement `evaluate_condition_group()` in `executor.rs`
    - [x] Add cycle detection to `loader.rs`
    - [x] Add depth validation to `loader.rs`
    - [x] Update CLI to display condition groups
    - [x] Add backward compatibility support for legacy conditions

11. **Phase 2: Expression Language** (3-4 days)
    - [x] Implement tokenizer in `expressions.rs`
    - [x] Implement parser in `expressions.rs`
    - [x] Implement evaluator with function library
    - [x] Implement string functions (upper, lower, trim, split, join, replace, length, matches)
    - [x] Implement number functions (abs, floor, ceil, round)
    - [x] Implement array functions (first, last, at, map, filter, sum, avg, min, max, count)
    - [x] Implement object functions (keys, values, get, has)
    - [x] Implement type functions (to_string, to_number, to_boolean, type)
    - [x] Integrate into executor (replace `render_value_with_outputs`)
    - [x] Add error handling and edge case support
    - [x] Update config schema for expression syntax

12. **Phase 3: New Step Types** (5-6 days)

    **3.1 Wait Step** (1 day)
    - [x] Add `WorkflowStepKind::Wait` variant
    - [x] Implement `execute_wait_step()`
    - [x] Add validation for wait config
    - [x] Support duration_seconds and until_expression modes

    **3.2 Loop Step** (2-3 days)
    - [x] Add `WorkflowStepKind::Loop` variant
    - [x] Implement `execute_loop_step()` with sequential mode
    - [x] Add parallel execution support
    - [x] Add iteration limit validation
    - [x] Implement loop variable scoping

    **3.3 Merge Step** (1 day)
    - [x] Add `WorkflowStepKind::Merge` variant
    - [x] Implement `execute_merge_step()`
    - [x] Support merge modes (merge, append, combine, multiplex)

    **3.4 Switch Step** (1 day)
    - [x] Add `WorkflowStepKind::Switch` variant
    - [x] Implement `execute_switch_step()`
    - [x] Integrate routing logic into main executor loop
    - [x] Support exact value and pattern matching

13. **Workflow Edge Cases & Error Handling** (2-3 days)
    - [x] Handle empty arrays (loop, sum, avg)
    - [x] Handle null/undefined values gracefully
    - [x] Add null handling configuration (strict vs lenient)
    - [x] Add depth limit enforcement (conditions, expressions)
    - [x] Detect circular references in step graphs
    - [x] Detect mutual workflow recursion
    - [x] Handle large data (iteration limits, streaming)
    - [x] Add error propagation with continue_on_error
    - [x] Implement graceful failure for loop iterations

### Phase 5: Tools Expansion (Weeks 13-19)
**Priority: HIGH - Covers all use cases from REQUIREMENTS**

14. **Gap 3: Tool Coverage Gap** (28-30 days total)

#### Priority 1: Essential Development Tools (Weeks 13-14)
    - [x] **3.1 Git Tool** (3 days)
      - [x] Create git.rs tool module
      - [x] Implement GitCommand enum (Clone, Pull, Push, Commit, Status, Diff, Branch, Tag, Log, Checkout)
      - [x] Use git2 crate for Rust bindings
      - [x] Implement streaming output for long operations
      - [x] Add permission checks (read-only for status/diff/log, read-write for others)
      - [x] Add working directory support
      - [x] Add error handling and context

    - [x] **3.2 Grep Tool** (2 days)
      - [x] Create grep.rs tool module
      - [x] Implement GrepArgs (pattern, path, glob, ignore_case, etc.)
      - [x] Use regex crate for pattern matching
      - [x] Use ignore crate for file walking with .gitignore
      - [x] Implement streaming results
      - [x] Add line numbers, context, max_results

    - [x] **3.3 Database Tool** (4 days)
      - [x] Create database.rs tool module
      - [x] Implement DatabaseCommand enum (Connect, Query, ListTables, DescribeTable)
      - [x] Add DatabaseType (Sqlite, Postgres, Mysql)
      - [x] Use sqlx for database abstraction
      - [x] Add connection pooling
      - [x] Implement query streaming
      - [x] Add timeout and cancellation support

#### Priority 2: Web & Search Tools (Week 15)
    - [x] **3.4 Web Search Tool** (2 days)
      - [x] Create web_search.rs tool module
      - [x] Implement WebSearchArgs (query, num_results, engine, lang)
      - [x] Add SearchEngine enum (Google, Bing, DuckDuckGo, Auto)
      - [x] Implement search via APIs
      - [x] Parse HTML responses
      - [x] Implement result ranking

    - [x] **3.5 Download Tool** (2 days)
      - [x] Create download.rs tool module
      - [x] Implement DownloadArgs (url, output, resume, chunk_size, max_size, timeout)
      - [x] Use reqwest with streaming
      - [x] Support range requests for resume
      - [x] Implement progress reporting via events
      - [x] Add size limits and SHA256 verification

#### Priority 3: Text & Data Tools (Week 16)
    - [x] **3.6 Regex Tool** (1 day)
      - [x] Create regex.rs tool module
      - [x] Implement RegexArgs and RegexOperation (Match, Replace, FindAll)
      - [x] Use regex crate with all flags
      - [x] Return structured matches with groups

    - [x] **3.7 Format Tool** (1 day)
      - [x] Create format.rs tool module
      - [x] Implement FormatArgs and FormatOperation (Json, Xml, Minify variants)
      - [x] Use serde_json for JSON
      - [x] Use quick-xml for XML
      - [x] Implement pretty printing and minify

    - [x] **3.8 Encoding Tool** (1 day)
      - [x] Create encoding.rs tool module
      - [x] Implement EncodingArgs and EncodingOperation (Base64, Url, HtmlEntities, ValidateUtf8)
      - [x] Use base64, percent-encoding, html-escape crates

    - [x] **3.9 Convert Tool** (2 days)
      - [x] Create convert.rs tool module
      - [x] Implement ConvertArgs (input, from, to) and DataFormat enum
      - [x] Use serde_json, serde_yaml, quick-xml, csv crates
      - [x] Use pulldown-cmark for MD->HTML
      - [x] Use html2md for HTML->MD

#### Priority 4: Code Intelligence (Weeks 17-18)
    - [x] **3.10 LSP Tool** (7 days)
      - [x] Create lsp.rs tool module
      - [x] Implement LspArgs and LspOperation (SymbolSearch, Definition, References, Hover)
      - [x] Add SymbolKind enum (Function, Method, Struct, etc.)
      - [x] Use lsp-types crate
      - [x] Implement LSP server communication via stdio
      - [x] Add server lifecycle management (start/stop)
      - [x] Implement caching for workspace symbols
      - [x] Implement search and navigation

#### Priority 5: Image Tools (Week 19)
    - [x] **3.11 Image Tool** (3 days)
      - [x] Create image.rs tool module
      - [x] Implement ImageArgs and ImageOperation (Resize, Crop, Rotate, Convert, Metadata)
      - [x] Add ImageFormat enum (Png, Jpeg, Webp, Gif)
      - [x] Use image crate for processing
      - [x] Implement resize, crop, rotate, format conversion
      - [x] Extract and return metadata
      - [x] Implement progress reporting

### Phase 6: Advanced Features (Weeks 20-24)
**Priority: MEDIUM - Continuous improvement**

15. **Gap 9: Self-Learning System** (14-18 days total)

#### Phase 1: Feedback Collection (Week 20)
    - [x] Create learning module (mod.rs, feedback.rs, storage.rs, types.rs)
    - [x] Implement UserFeedback structure (id, session_id, agent_name, feedback_type, rating, comment, context)
    - [x] Implement FeedbackContext (task_description, tools_used, model_response, error_occurred, error_message)
    - [x] Add SQLite table for feedback storage
    - [x] Create migration script for feedback table
    - [x] Implement feedback collection API
    - [x] Add CLI command: `/feedback --type <type> --rating <-1..1> --comment <text>`
    - [x] Add implicit feedback collection from events

#### Phase 2: Mistake Pattern Learning (Weeks 21-22)
    - [x] Create pattern learning module (patterns.rs)
    - [x] Implement MistakePattern structure (pattern_type, trigger, frequency, last_seen, suggested_fix)
    - [x] Implement MistakeType enum (PermissionDenied, ToolTimeout, FileNotFound, CompilationError, TestFailure, WrongApproach)
    - [x] Implement PatternLearner
    - [x] Implement event recording (ToolFailed, PermissionDenied)
    - [x] Implement error classification (classify_error method)
    - [x] Implement pattern frequency tracking
    - [x] Implement fix suggestion (suggest_fix method)
    - [x] Implement active pattern retrieval (get_active_patterns)
    - [x] Implement pattern-based warnings before task execution
    - [x] Add SQLite table for mistake patterns
    - [x] Create migration script for patterns table

#### Phase 3: Preference Adaptation (Week 23)
    - [x] Create preference learning module (preferences.rs)
    - [x] Implement UserPreferences structure (id, session_id, preferences HashMap)
    - [x] Implement PreferenceValue enum (String, Int, Float, Bool)
    - [x] Implement PreferenceLearner
    - [x] Implement choice recording (record_choice)
    - [x] Implement rating recording (record_rating)
    - [x] Implement preference retrieval (get_preference)
    - [x] Implement preferred approach retrieval (get_preferred_approach)
    - [x] Add SQLite table for user preferences
    - [x] Create migration script for preferences table

#### Phase 4: Success Pattern Library (Week 24)
    - [x] Create success patterns module (success_patterns.rs)
    - [x] Implement SuccessPattern structure (id, name, category, description, template, frequency, last_used, success_rate)
    - [x] Implement PatternCategory enum (ErrorFixing, Refactoring, Debugging, FeatureImplementation, Testing)
    - [x] Implement SuccessPatternLibrary
    - [x] Implement success recording (record_success)
    - [x] Implement pattern similarity scoring (similarity method)
    - [x] Implement pattern finding (find_patterns)
    - [x] Implement top pattern retrieval (get_top_patterns)
    - [x] Implement name generation (generate_name)
    - [x] Implement template extraction (extract_template)
    - [x] Add SQLite table for success patterns
    - [x] Create migration script for success patterns table

#### Phase 5: Integration with Agent
    - [x] Modify Agent::run to incorporate learning
    - [x] Add pattern check before task execution (show known patterns)
    - [x] Record task completion for success patterns
    - [x] Record errors for mistake patterns
    - [x] Apply user preferences in agent behavior
    - [x] Emit learning-related events

### Phase 7: Big Codebase Support (Weeks 25-28)
**Priority: CRITICAL - Required for large codebases (100K+ files)**

16. **Gap 8: Big Codebase Support (RAG, Vector DB, Indexing)** (26-30 days total)

#### Phase 1: Code Indexing (Weeks 25-26)
    - [x] Create indexing module (mod.rs, parser.rs, symbols.rs, types.rs)
    - [x] Implement CodeIndex structure (workspace, files, symbols, dependencies, updated_at)
    - [x] Implement FileIndex structure (path, language, functions, classes, imports)
    - [x] Implement SymbolIndex structure (name, symbol_type, file_path, line, column, docstring, signature)
    - [x] Implement SymbolType enum (Function, Method, Struct, Enum, Trait, Impl, Type, Variable, Constant, Module)
    - [x] Add tree-sitter for multi-language parsing
    - [x] Support Rust, Python, JavaScript/TypeScript, Go, C/C++
    - [x] Implement AST node extraction for symbols
    - [x] Build call graph tracking
    - [x] Add SQLite tables for code index
    - [x] Create migration scripts for index tables
    - [x] Implement incremental index updates

#### Phase 2: Vector Database Integration (Weeks 26-27)
    - [x] Create vector module (mod.rs, db.rs, embedding.rs)
    - [x] Implement VectorDB with VectorBackend (SqliteVector, PostgresVector, etc.)
    - [x] Implement EmbeddingProvider trait (embed, dimension)
    - [x] Implement OpenAI embedding provider (text-embedding-3-small/large)
    - [x] Implement OpenAI-compatible local embedding provider
    - [x] Implement sentence-transformers provider (local Python)
    - [x] Implement Embedding structure (id, vector, metadata)
    - [x] Implement SearchQuery structure (text, top_k, filter)
    - [x] Implement SearchResult structure (id, score, metadata)
    - [x] Add vector-sqlite extension integration
    - [x] Implement vector storage and retrieval
    - [x] Implement cosine similarity search
    - [x] Add SQLite tables for vectors
    - [x] Create migration scripts for vector tables

#### Phase 3: RAG System (Weeks 27-28)
    - [x] Create rag module (mod.rs, retriever.rs, augmenter.rs)
    - [x] Implement RAGRetriever (index, vector_db)
    - [x] Implement RetrievalRequest (query, top_k, min_score, filters)
    - [x] Implement RetrievalResult (snippets, symbols)
    - [x] Implement CodeSnippet structure (file_path, content, line_start, line_end, score, contexts)
    - [x] Implement SymbolMatch structure (symbol, score, usage_context)
    - [x] Implement hybrid search (keyword + semantic)
    - [x] Implement context expansion (surrounding lines)
    - [x] Implement result ranking (relevance, recency, importance)
    - [x] Implement prompt augmentation
    - [x] Implement code context formatting

#### Phase 4: Integration with Agent
    - [x] Modify Agent::build_context_window to use RAG
    - [x] Implement RAG-based context building
    - [x] Extract last query from conversation
    - [x] Retrieve relevant code snippets
    - [x] Build code context from retrieval
    - [x] Inject code context as system message
    - [x] Limit conversation history to fit RAG context
    - [x] Implement token budget management for RAG

---

## Release Preparation

17. **Update README.md** (1 day)
    - [ ] Fix "scaffold + foundation stage" claim
    - [ ] Reflect completion status
    - [ ] Add quick start examples
    - [ ] Update feature matrix

18. **Advanced Tools** (5-7 days)
    - [ ] Git tool (clone, commit, push, pull, status, diff, log, branch)
    - [ ] Database tool (query, list tables, describe table)
    - [ ] LSP client tool (symbol search, definition, references, hover)

19. **Code Graph Analysis** (7-10 days)
    - [ ] AST parsing/indexing
    - [ ] Dependency tracking
    - [ ] Impact analysis for changes
    - [ ] Code visualization

20. **API Server** (7-10 days)
    - [ ] REST API for programmatic access
    - [ ] SSE/WebSocket API for streaming
    - [ ] Remote execution capability
    - [ ] Multi-user support
    - [ ] API authentication

21. **TUI Frontend** (10-14 days)
    - [ ] Terminal UI using ratatui or similar
    - [ ] Interactive workflow builder
    - [ ] Better than current text-only REPL
    - [ ] Multi-panel layout

22. **Add Comprehensive Tests** (7-10 days)
    - [ ] Add unit tests for all core modules (config, providers, tools, workflows, agents, permissions, storage)
    - [ ] Add integration tests for provider integrations (all 7 providers)
    - [ ] Add integration tests for tool execution (all tools with streaming)
    - [ ] Add integration tests for workflow execution (all step types, conditions, expressions)
    - [ ] Add integration tests for agent coordination (tool calling loops, sub-agent calls)
    - [ ] Add integration tests for permission enforcement (allow/deny/ask, read/write)
    - [ ] Add integration tests for storage backends (SQLite, PostgreSQL)
    - [ ] Add E2E tests for complex workflows
    - [ ] Add E2E tests for multi-agent scenarios
    - [ ] Add performance tests (large workflows, big codebases)
    - [ ] Add load tests (concurrent sessions, parallel workflows)
    - [ ] Target: 80%+ code coverage
    - [ ] Ensure all tests pass

---

## Release

- [ ] Release v1.0.0
    - [ ] Update version numbers
    - [ ] Generate CHANGELOG.md
    - [ ] Create release notes
    - [ ] Tag release in git
    - [ ] Publish to crates.io
    - [ ] Create GitHub release
    - [ ] Announce in relevant communities

---

## Current Focus (Continuing)

- [ ] Workflow enhancements (from previous TODO)
    - [x] Phase 1: Grouped conditions with nested logic (2-3 days)
    - [x] Phase 2: Expression language hardening (error model + schema update)
    - [x] Phase 3: New step types follow-up (loop parallel mode, switch pattern matching)

---

## Next (Immediate)

- [x] Add timeout policy controls to route step timeout as failure when configured
- [x] Add per-step cancel-signal integration for long-running tool/plugin executions
- [x] Surface workflow retry/timeout aggregates in CLI workflow run summaries
- [x] Implement full OpenCode-parity interrupt: user-triggered cancellation for active agent and sub-agent turns (not only tool/process timeout cancellation)

---

## Completed (Migrated from previous TODO)

- [x] Initialize workspace (`rustic-ai-core`, `frontend/rustic-ai-cli`)
- [x] Build/lint/test baseline green for initial foundation
- [x] Implement core config loading, merge, and validation framework
- [x] Implement config mutation layer (`ConfigManager`, typed paths, atomic writes)
- [x] Implement storage abstraction and SQLite backend with session/message persistence
- [x] Implement provider registry and factory boundary
- [x] Implement OpenAI provider baseline (`generate`)
- [x] Implement Anthropic, Google, Grok, Z.ai, Ollama, Custom providers
- [x] Implement subscription auth subsystem (OAuth PKCE, device-code)
- [x] Implement tool system foundation (`Tool` trait, registry, manager)
- [x] Implement Shell, Filesystem, HTTP, SSH tools
- [x] Implement permission policy foundation (`allow` / `deny` / `ask`)
- [x] Implement agent/session flow foundation and CLI interactive loop
- [x] Wire agent turn loop to handle permission-approved follow-up
- [x] Implement Filesystem, HTTP tools
- [x] Implement agent tool-call orchestration
- [x] Implement SSH persistent session tool
- [x] Implement MCP adapter integration
- [x] Implement plugin tool loader wiring
- [x] Config ergonomics (layered configs, split-file layouts)
- [x] Implement skills + workflows foundation (n8n-oriented)
- [x] Implement workflow execution engine with named outputs
- [x] Add CLI commands for skills/workflows/permissions
- [x] Document comprehensive workflow enhancement plan
- [x] Document comprehensive state analysis
- [x] Document implementation gaps (all 9 gaps with full plans)
- [x] Document agent-to-agent calling protocol

---

## Verification Commands
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt --all`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --workspace`
- `export PATH="$HOME/.cargo/bin:$PATH" && cargo clippy --workspace --all-targets --all-features -- -D warnings`

---

## Update Rules
- Every non-trivial change updates this file in same change.
- Keep only one active tracker (this file).
- Move completed work to Done immediately.
- Reflect scope changes before implementation proceeds.

---

## Reference Documents

- **Requirements:** `docs/initial-planning/REQUIREMENTS.md`
- **Implementation Gaps:** `docs/implementation-gaps.md` (comprehensive gap analysis)
- **Workflow Plan:** `docs/workflow-enhancement-plan.md`
- **State Analysis:** `docs/comprehensive-state-analysis.md`
- **Agent Protocol:** `docs/agent-to-agent-protocol.md` (OpenCode-style agent calling)
- **Design Guide:** `docs/DESIGN_GUIDE.md`
- **Decisions:** `docs/DECISIONS.md`
- **Integration Plan:** `docs/initial-planning/integration-plan.md`
- **Big Picture:** `docs/initial-planning/big-picture.md`
- **Tools:** `docs/initial-planning/tools.md`
