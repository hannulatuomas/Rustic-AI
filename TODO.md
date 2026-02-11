# TODO.md

Single source of truth for active implementation work.

Last updated: 2026-02-11

## Project Status

**Overall Requirements Coverage: ~52%**

**Current Implementation vs Requirements:**
- LLM Providers: 175% (7 implemented, 4 required)
- Tool System: 23% (7 implemented, 30+ required)
- Permissions: 50% (allow/deny/ask, missing read/write)
- Agent Coordination: 30% (partial, missing direct agent calling)
- Context Handling: 20% (simple LIFO, no summarization/RAG)
- Big Codebases: 0% (no indexing, vector DB, semantic search)
- Self-Learning: 0% (no feedback, patterns, adaptation)
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
    - [ ] Wire workflow trigger execution
    - [ ] Implement cron scheduling
    - [ ] Implement event-based triggers
    - [ ] Integrate topic inference into agent prompts
    - [ ] Implement topic inference completion
    - [ ] Implement project mode rule scoping
    - [ ] Complete project mode profile loading
    - [ ] Wire PostgreSQL backend
    - [ ] Implement PostgreSQL storage backend
    - [ ] Add custom storage backend support

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
    - [ ] Add `LogicalOperator`, `Condition`, `ConditionGroup` types to `types.rs`
    - [ ] Implement `evaluate_condition_group()` in `executor.rs`
    - [ ] Add cycle detection to `loader.rs`
    - [ ] Add depth validation to `loader.rs`
    - [ ] Update CLI to display condition groups
    - [ ] Add backward compatibility support for legacy conditions

11. **Phase 2: Expression Language** (3-4 days)
    - [ ] Implement tokenizer in `expressions.rs`
    - [ ] Implement parser in `expressions.rs`
    - [ ] Implement evaluator with function library
    - [ ] Implement string functions (upper, lower, trim, split, join, replace, length, matches)
    - [ ] Implement number functions (abs, floor, ceil, round)
    - [ ] Implement array functions (first, last, at, map, filter, sum, avg, min, max, count)
    - [ ] Implement object functions (keys, values, get, has)
    - [ ] Implement type functions (to_string, to_number, to_boolean, type)
    - [ ] Integrate into executor (replace `render_value_with_outputs`)
    - [ ] Add error handling and edge case support
    - [ ] Update config schema for expression syntax

12. **Phase 3: New Step Types** (5-6 days)

    **3.1 Wait Step** (1 day)
    - [ ] Add `WorkflowStepKind::Wait` variant
    - [ ] Implement `execute_wait_step()`
    - [ ] Add validation for wait config
    - [ ] Support duration_seconds and until_expression modes

    **3.2 Loop Step** (2-3 days)
    - [ ] Add `WorkflowStepKind::Loop` variant
    - [ ] Implement `execute_loop_step()` with sequential mode
    - [ ] Add parallel execution support
    - [ ] Add iteration limit validation
    - [ ] Implement loop variable scoping

    **3.3 Merge Step** (1 day)
    - [ ] Add `WorkflowStepKind::Merge` variant
    - [ ] Implement `execute_merge_step()`
    - [ ] Support merge modes (merge, append, combine, multiplex)

    **3.4 Switch Step** (1 day)
    - [ ] Add `WorkflowStepKind::Switch` variant
    - [ ] Implement `execute_switch_step()`
    - [ ] Integrate routing logic into main executor loop
    - [ ] Support exact value and pattern matching

13. **Workflow Edge Cases & Error Handling** (2-3 days)
    - [ ] Handle empty arrays (loop, sum, avg)
    - [ ] Handle null/undefined values gracefully
    - [ ] Add null handling configuration (strict vs lenient)
    - [ ] Add depth limit enforcement (conditions, expressions)
    - [ ] Detect circular references in step graphs
    - [ ] Detect mutual workflow recursion
    - [ ] Handle large data (iteration limits, streaming)
    - [ ] Add error propagation with continue_on_error
    - [ ] Implement graceful failure for loop iterations

### Phase 5: Tools Expansion (Weeks 13-19)
**Priority: HIGH - Covers all use cases from REQUIREMENTS**

14. **Gap 3: Tool Coverage Gap** (28-30 days total)

#### Priority 1: Essential Development Tools (Weeks 13-14)
    - [ ] **3.1 Git Tool** (3 days)
      - [ ] Create git.rs tool module
      - [ ] Implement GitCommand enum (Clone, Pull, Push, Commit, Status, Diff, Branch, Tag, Log, Checkout)
      - [ ] Use git2 crate for Rust bindings
      - [ ] Implement streaming output for long operations
      - [ ] Add permission checks (read-only for status/diff/log, read-write for others)
      - [ ] Add working directory support
      - [ ] Add error handling and context

    - [ ] **3.2 Grep Tool** (2 days)
      - [ ] Create grep.rs tool module
      - [ ] Implement GrepArgs (pattern, path, glob, ignore_case, etc.)
      - [ ] Use regex crate for pattern matching
      - [ ] Use ignore crate for file walking with .gitignore
      - [ ] Implement streaming results
      - [ ] Add line numbers, context, max_results

    - [ ] **3.3 Database Tool** (4 days)
      - [ ] Create database.rs tool module
      - [ ] Implement DatabaseCommand enum (Connect, Query, ListTables, DescribeTable)
      - [ ] Add DatabaseType (Sqlite, Postgres, Mysql)
      - [ ] Use sqlx for database abstraction
      - [ ] Add connection pooling
      - [ ] Implement query streaming
      - [ ] Add timeout and cancellation support

#### Priority 2: Web & Search Tools (Week 15)
    - [ ] **3.4 Web Search Tool** (2 days)
      - [ ] Create web_search.rs tool module
      - [ ] Implement WebSearchArgs (query, num_results, engine, lang)
      - [ ] Add SearchEngine enum (Google, Bing, DuckDuckGo, Auto)
      - [ ] Implement search via APIs
      - [ ] Parse HTML responses
      - [ ] Implement result ranking

    - [ ] **3.5 Download Tool** (2 days)
      - [ ] Create download.rs tool module
      - [ ] Implement DownloadArgs (url, output, resume, chunk_size, max_size, timeout)
      - [ ] Use reqwest with streaming
      - [ ] Support range requests for resume
      - [ ] Implement progress reporting via events
      - [ ] Add size limits and SHA256 verification

#### Priority 3: Text & Data Tools (Week 16)
    - [ ] **3.6 Regex Tool** (1 day)
      - [ ] Create regex.rs tool module
      - [ ] Implement RegexArgs and RegexOperation (Match, Replace, FindAll)
      - [ ] Use regex crate with all flags
      - [ ] Return structured matches with groups

    - [ ] **3.7 Format Tool** (1 day)
      - [ ] Create format.rs tool module
      - [ ] Implement FormatArgs and FormatOperation (Json, Xml, Minify variants)
      - [ ] Use serde_json for JSON
      - [ ] Use quick-xml for XML
      - [ ] Implement pretty printing and minify

    - [ ] **3.8 Encoding Tool** (1 day)
      - [ ] Create encoding.rs tool module
      - [ ] Implement EncodingArgs and EncodingOperation (Base64, Url, HtmlEntities, ValidateUtf8)
      - [ ] Use base64, percent-encoding, html-escape crates

    - [ ] **3.9 Convert Tool** (2 days)
      - [ ] Create convert.rs tool module
      - [ ] Implement ConvertArgs (input, from, to) and DataFormat enum
      - [ ] Use serde_json, serde_yaml, quick-xml, csv crates
      - [ ] Use pulldown-cmark for MD->HTML
      - [ ] Use html2md for HTML->MD

#### Priority 4: Code Intelligence (Weeks 17-18)
    - [ ] **3.10 LSP Tool** (7 days)
      - [ ] Create lsp.rs tool module
      - [ ] Implement LspArgs and LspOperation (SymbolSearch, Definition, References, Hover)
      - [ ] Add SymbolKind enum (Function, Method, Struct, etc.)
      - [ ] Use lsp-types crate
      - [ ] Implement LSP server communication via stdio
      - [ ] Add server lifecycle management (start/stop)
      - [ ] Implement caching for workspace symbols
      - [ ] Implement search and navigation

#### Priority 5: Image Tools (Week 19)
    - [ ] **3.11 Image Tool** (3 days)
      - [ ] Create image.rs tool module
      - [ ] Implement ImageArgs and ImageOperation (Resize, Crop, Rotate, Convert, Metadata)
      - [ ] Add ImageFormat enum (Png, Jpeg, Webp, Gif)
      - [ ] Use image crate for processing
      - [ ] Implement resize, crop, rotate, format conversion
      - [ ] Extract and return metadata
      - [ ] Implement progress reporting

### Phase 6: Advanced Features (Weeks 20-24)
**Priority: MEDIUM - Continuous improvement**

15. **Gap 9: Self-Learning System** (14-18 days total)

#### Phase 1: Feedback Collection (Week 20)
    - [ ] Create learning module (mod.rs, feedback.rs, storage.rs, types.rs)
    - [ ] Implement UserFeedback structure (id, session_id, agent_name, feedback_type, rating, comment, context)
    - [ ] Implement FeedbackContext (task_description, tools_used, model_response, error_occurred, error_message)
    - [ ] Add SQLite table for feedback storage
    - [ ] Create migration script for feedback table
    - [ ] Implement feedback collection API
    - [ ] Add CLI command: `/feedback --type <type> --rating <-1..1> --comment <text>`
    - [ ] Add implicit feedback collection from events

#### Phase 2: Mistake Pattern Learning (Weeks 21-22)
    - [ ] Create pattern learning module (patterns.rs)
    - [ ] Implement MistakePattern structure (pattern_type, trigger, frequency, last_seen, suggested_fix)
    - [ ] Implement MistakeType enum (PermissionDenied, ToolTimeout, FileNotFound, CompilationError, TestFailure, WrongApproach)
    - [ ] Implement PatternLearner
    - [ ] Implement event recording (ToolFailed, PermissionDenied)
    - [ ] Implement error classification (classify_error method)
    - [ ] Implement pattern frequency tracking
    - [ ] Implement fix suggestion (suggest_fix method)
    - [ ] Implement active pattern retrieval (get_active_patterns)
    - [ ] Implement pattern-based warnings before task execution
    - [ ] Add SQLite table for mistake patterns
    - [ ] Create migration script for patterns table

#### Phase 3: Preference Adaptation (Week 23)
    - [ ] Create preference learning module (preferences.rs)
    - [ ] Implement UserPreferences structure (id, session_id, preferences HashMap)
    - [ ] Implement PreferenceValue enum (String, Int, Float, Bool)
    - [ ] Implement PreferenceLearner
    - [ ] Implement choice recording (record_choice)
    - [ ] Implement rating recording (record_rating)
    - [ ] Implement preference retrieval (get_preference)
    - [ ] Implement preferred approach retrieval (get_preferred_approach)
    - [ ] Add SQLite table for user preferences
    - [ ] Create migration script for preferences table

#### Phase 4: Success Pattern Library (Week 24)
    - [ ] Create success patterns module (success_patterns.rs)
    - [ ] Implement SuccessPattern structure (id, name, category, description, template, frequency, last_used, success_rate)
    - [ ] Implement PatternCategory enum (ErrorFixing, Refactoring, Debugging, FeatureImplementation, Testing)
    - [ ] Implement SuccessPatternLibrary
    - [ ] Implement success recording (record_success)
    - [ ] Implement pattern similarity scoring (similarity method)
    - [ ] Implement pattern finding (find_patterns)
    - [ ] Implement top pattern retrieval (get_top_patterns)
    - [ ] Implement name generation (generate_name)
    - [ ] Implement template extraction (extract_template)
    - [ ] Add SQLite table for success patterns
    - [ ] Create migration script for success patterns table

#### Phase 5: Integration with Agent
    - [ ] Modify Agent::run to incorporate learning
    - [ ] Add pattern check before task execution (show known patterns)
    - [ ] Record task completion for success patterns
    - [ ] Record errors for mistake patterns
    - [ ] Apply user preferences in agent behavior
    - [ ] Emit learning-related events

### Phase 7: Big Codebase Support (Weeks 25-28)
**Priority: CRITICAL - Required for large codebases (100K+ files)**

16. **Gap 8: Big Codebase Support (RAG, Vector DB, Indexing)** (26-30 days total)

#### Phase 1: Code Indexing (Weeks 25-26)
    - [ ] Create indexing module (mod.rs, parser.rs, symbols.rs, types.rs)
    - [ ] Implement CodeIndex structure (workspace, files, symbols, dependencies, updated_at)
    - [ ] Implement FileIndex structure (path, language, functions, classes, imports)
    - [ ] Implement SymbolIndex structure (name, symbol_type, file_path, line, column, docstring, signature)
    - [ ] Implement SymbolType enum (Function, Method, Struct, Enum, Trait, Impl, Type, Variable, Constant, Module)
    - [ ] Add tree-sitter for multi-language parsing
    - [ ] Support Rust, Python, JavaScript/TypeScript, Go, C/C++
    - [ ] Implement AST node extraction for symbols
    - [ ] Build call graph tracking
    - [ ] Add SQLite tables for code index
    - [ ] Create migration scripts for index tables
    - [ ] Implement incremental index updates

#### Phase 2: Vector Database Integration (Weeks 26-27)
    - [ ] Create vector module (mod.rs, db.rs, embedding.rs)
    - [ ] Implement VectorDB with VectorBackend (SqliteVector, PostgresVector, etc.)
    - [ ] Implement EmbeddingProvider trait (embed, dimension)
    - [ ] Implement OpenAI embedding provider (text-embedding-3-small/large)
    - [ ] Implement OpenAI-compatible local embedding provider
    - [ ] Implement sentence-transformers provider (local Python)
    - [ ] Implement Embedding structure (id, vector, metadata)
    - [ ] Implement SearchQuery structure (text, top_k, filter)
    - [ ] Implement SearchResult structure (id, score, metadata)
    - [ ] Add vector-sqlite extension integration
    - [ ] Implement vector storage and retrieval
    - [ ] Implement cosine similarity search
    - [ ] Add SQLite tables for vectors
    - [ ] Create migration scripts for vector tables

#### Phase 3: RAG System (Weeks 27-28)
    - [ ] Create rag module (mod.rs, retriever.rs, augmenter.rs)
    - [ ] Implement RAGRetriever (index, vector_db)
    - [ ] Implement RetrievalRequest (query, top_k, min_score, filters)
    - [ ] Implement RetrievalResult (snippets, symbols)
    - [ ] Implement CodeSnippet structure (file_path, content, line_start, line_end, score, contexts)
    - [ ] Implement SymbolMatch structure (symbol, score, usage_context)
    - [ ] Implement hybrid search (keyword + semantic)
    - [ ] Implement context expansion (surrounding lines)
    - [ ] Implement result ranking (relevance, recency, importance)
    - [ ] Implement prompt augmentation
    - [ ] Implement code context formatting

#### Phase 4: Integration with Agent
    - [ ] Modify Agent::build_context_window to use RAG
    - [ ] Implement RAG-based context building
    - [ ] Extract last query from conversation
    - [ ] Retrieve relevant code snippets
    - [ ] Build code context from retrieval
    - [ ] Inject code context as system message
    - [ ] Limit conversation history to fit RAG context
    - [ ] Implement token budget management for RAG

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
    - [ ] Phase 1: Grouped conditions with nested logic (2-3 days)
    - [ ] Phase 2: Expression language (3-4 days)
    - [ ] Phase 3: New step types (Wait, Loop, Merge, Switch) (4-6 days)

---

## Next (Immediate)

- Start with Phase 1 (Foundation - Agent Capabilities)
- Prioritize by impact and dependencies

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
