# Implementation Gaps Analysis & Plan

**Date:** 2026-02-11
**Purpose:** Document all identified gaps vs. REQUIREMENTS.md and create implementation plans

---

## Executive Summary

This document identifies all gaps between current Rustic-AI implementation and the requirements specified in `docs/initial-planning/REQUIREMENTS.md`. Each gap includes:

1. Current state (what exists)
2. What's missing
3. Why it matters
4. Implementation plan with dependencies
5. Estimated effort

**Overall Coverage Assessment:**

| Requirement Category | Required Features | Implemented | Coverage |
|---------------------|------------------|-------------|----------|
| **LLM Providers** | 4+ providers | 7 providers | ✅ 175% |
| **Tool System** | 30+ tools | 7 tools | ⚠️ 23% |
| **Permissions** | Allow/deny/ask, read/write | allow/deny/ask only | ⚠️ 50% |
| **Agent Coordination** | Multi-agent, sub-agents | Partial (workflows only) | ⚠️ 30% |
| **Context Handling** | Pruning, summarization, RAG | Simple LIFO only | ❌ 20% |
| **Big Codebases** | Indexing, vector DB, semantic search | None | ❌ 0% |
| **Self-Learning** | Feedback, patterns, adaptation | None | ❌ 0% |
| **Taxonomy** | Baskets for organization | Schema only | ❌ 10% |

**Overall Requirements Coverage: ~52%**

---

## Gap 1: Agent Permission Model (Read-Only vs Read-Write)

### Current State

**What exists:**
- Permission system with three modes: `Allow`, `Deny`, `Ask` (from `permissions/policy.rs`)
- `Ask` can resolve to: `AllowOnce`, `AllowInSession`, `Deny`
- Each agent has a `tools: Vec<String>` whitelist
- Each tool has a `permission_mode: PermissionMode`
- Permission decisions are persisted with scope (session/project/global)

**What's missing:**
- **No concept of read-only vs read-write permissions for agents**
- No agent-level permission modes
- No distinction between:
  - Planner agents (should be read-only)
  - Builder agents (should be read-write)
  - Reviewer agents (should be read-only)

### Why It Matters

From `REQUIREMENTS.md:92-93`:
- Security: "Permission management"
- "Secure execution of commands"

Without read/write distinction:
- Planner agents could accidentally modify files
- No way to enforce least privilege for agent roles
- Can't safely delegate to review/audit agents without risk of modification

### Implementation Plan

#### Phase 1: Extend Permission Model

**Files to modify:**
1. `rustic-ai-core/src/permissions/policy.rs`
2. `rustic-ai-core/src/config/schema.rs`

**Changes:**

```rust
// Add to policy.rs
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionMode {
    /// Agent can only read data, not modify
    Read,
    /// Agent can read and write data
    ReadWrite,
    /// Agent's permissions are determined by tool-level settings (default)
    Inherit,
}
```

```rust
// Extend PermissionDecision to capture operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDecision {
    pub action: PermissionAction,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionAction {
    Allow,
    Deny,
    Ask,
    AllowRead,      // Allow read operations only
    AllowWrite,     // Allow write operations only
}
```

#### Phase 2: Add Agent-Level Permission Config

**Modify:**
- `rustic-ai-core/src/config/schema.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    // ... existing fields ...

    /// Agent's permission mode: Read, ReadWrite, or Inherit (default)
    pub permission_mode: AgentPermissionMode,
}
```

#### Phase 3: Enforce Read-Only in Tools

**Modify:**
- `rustic-ai-core/src/tools/filesystem.rs`
- `rustic-ai-core/src/tools/shell.rs`
- `rustic-ai-core/src/tools/ssh.rs`
- `rustic-ai-core/src/tools/manager.rs`

**Approach:**
1. Add `OperationType` to tool execution context:
   ```rust
   pub enum ToolOperationType {
       Read,    // read, list, info, glob, hash, diff
       Write,   // write, edit, copy, move, delete, mkdir
       Exec,    // shell execution, ssh, http requests
   }
   ```

2. Extend `ToolExecutionContext`:
   ```rust
   pub struct ToolExecutionContext {
       pub session_id: String,
       pub agent_name: Option<String>,
       pub working_directory: PathBuf,
       pub agent_permission_mode: AgentPermissionMode,
   }
   ```

3. Check permissions in tool methods:
   ```rust
   async fn read(&self, args: ReadArgs, ctx: &ToolExecutionContext) -> Result<ToolResult> {
       // Read is always allowed for Read and ReadWrite
       if matches!(ctx.agent_permission_mode, AgentPermissionMode::Read | AgentPermissionMode::ReadWrite) {
           // proceed
       } else {
           return Err(Error::Permission("read operation denied: agent is read-only".into()));
       }
   }

   async fn write(&self, args: WriteArgs, ctx: &ToolExecutionContext) -> Result<ToolResult> {
       // Write only allowed for ReadWrite
       if matches!(ctx.agent_permission_mode, AgentPermissionMode::ReadWrite) {
           // proceed
       } else {
           return Err(Error::Permission("write operation denied: agent is read-only".into()));
       }
   }
   ```

#### Phase 4: Update ToolManager

**Modify:**
- `rustic-ai-core/src/tools/manager.rs`

**Changes:**
1. Get agent's permission mode when executing tool
2. Pass it to execution context
3. Emit permission check events for read/write violations

#### Phase 5: CLI/UI Updates

**Modify:**
- `frontend/rustic-ai-cli/src/render.rs`

**Changes:**
1. Show agent permission mode in `/agents list` output
2. Show permission check events when agent violates read-only
3. Update `/agents show` to display permission mode

**Estimated Effort:** 4-5 days

**Dependencies:** None

---

## Gap 2: Taxonomy Implementation (Baskets for Organization)

### Current State

**What exists:**
- Config schema defines `TaxonomyConfig` with `BasketConfig` (from `config/schema.rs:572-582`)
- Schema structure:
  ```rust
  pub struct TaxonomyConfig {
      pub baskets: Vec<BasketConfig>,
  }

  pub struct BasketConfig {
      pub name: String,
      pub sub_baskets: Vec<String>,
  }
  ```

**What's missing:**
- `AgentRegistry` is essentially empty (`pub struct AgentRegistry;`)
- No code to:
  - Load taxonomy from config
  - Associate agents/tools/skills with baskets
  - Provide APIs for filtering by basket
  - Use taxonomy for discovery or routing

From `big-picture.md:183-189`:
- Purpose: "Basket taxonomy for discoverability and UX"
- "Depth 2 hierarchy only: Basket -> Sub-basket"
- "Items can belong to multiple baskets/sub-baskets"
- "Taxonomy is metadata for discovery and filtering, not execution policy"

### Why It Matters

Without taxonomy implementation:
- Large collections of agents/tools become unmanageable
- No way to organize by domain (e.g., "Development", "DevOps", "Security")
- Poor UX for discovery: users must scan through flat lists
- Can't implement "planner" vs "builder" organization in UI

### Implementation Plan

#### Phase 1: Extend Config Schema with Membership

**Modify:**
- `rustic-ai-core/src/config/schema.rs`

**Changes:**

```rust
// Add to AgentConfig
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    // ... existing fields ...

    /// Taxonomy membership: list of (basket, sub_basket) tuples
    pub taxonomy_membership: Vec<(String, Option<String>)>,
}
```

```rust
// Same for ToolConfig and skill manifest structures
```

#### Phase 2: Implement Taxonomy Registry

**Create:**
- `rustic-ai-core/src/taxonomy/mod.rs`
- `rustic-ai-core/src/taxonomy/registry.rs`
- `rustic-ai-core/src/taxonomy/types.rs`

**Files:**

`taxonomy/types.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taxonomy {
    baskets: Vec<Basket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Basket {
    name: String,
    sub_baskets: Vec<SubBasket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubBasket {
    name: String,
    items: Vec<TaxonomyItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyItem {
    id: String,
    item_type: ItemType,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Agent,
    Tool,
    Skill,
}
```

`taxonomy/registry.rs`:
```rust
use crate::config::schema::TaxonomyConfig;
use super::types::{Taxonomy, Basket, SubBasket, ItemType};

pub struct TaxonomyRegistry {
    taxonomy: Taxonomy,
}

impl TaxonomyRegistry {
    pub fn new(config: &TaxonomyConfig) -> Result<Self> {
        let taxonomy = Self::load_taxonomy(config)?;
        Ok(Self { taxonomy })
    }

    fn load_taxonomy(config: &TaxonomyConfig) -> Result<Taxonomy> {
        // Build taxonomy from config.baskets
        // Validate depth (max 2)
        // Check for cycles
    }

    // Query APIs
    pub fn get_basket(&self, name: &str) -> Option<&Basket> {
        self.taxonomy.baskets.iter().find(|b| b.name == name)
    }

    pub fn get_sub_basket(&self, basket_name: &str, sub_name: &str) -> Option<&SubBasket> {
        self.get_basket(basket_name)?
            .sub_baskets.iter()
            .find(|s| s.name == sub_name)
    }

    pub fn filter_by_basket(&self, basket_name: &str) -> Vec<TaxonomyItem> {
        self.get_basket(basket_name)
            .map(|basket| {
                basket.sub_baskets.iter()
                    .flat_map(|sb| sb.items.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn filter_by_sub_basket(&self, basket_name: &str, sub_name: &str) -> Vec<TaxonomyItem> {
        self.get_sub_basket(basket_name, sub_name)
            .map(|sb| sb.items.clone())
            .unwrap_or_default()
    }

    pub fn search_items(&self, query: &str) -> Vec<TaxonomyItem> {
        self.taxonomy.baskets.iter()
            .flat_map(|basket| basket.sub_baskets.iter())
            .flat_map(|sb| sb.items.iter())
            .filter(|item| {
                item.name.to_lowercase().contains(&query.to_lowercase())
                    || item.description.as_ref()
                        .map(|d| d.to_lowercase().contains(&query.to_lowercase()))
                        .unwrap_or(false)
            })
            .cloned()
            .collect()
    }
}
```

#### Phase 3: Wire Taxonomy into Initialization

**Modify:**
- `rustic-ai-core/src/lib.rs` or initialization module

**Changes:**
1. Create `TaxonomyRegistry` from config
2. Pass to registries for filtering

```rust
pub struct RusticAI {
    // ... existing fields ...
    taxonomy_registry: Arc<TaxonomyRegistry>,
}
```

#### Phase 4: Update Registries to Use Taxonomy

**Modify:**
- `rustic-ai-core/src/agents/registry.rs`
- `rustic-ai-core/src/tools/registry.rs`

**Changes:**
```rust
pub struct AgentRegistry {
    agents: HashMap<String, Arc<Agent>>,
    taxonomy: Arc<TaxonomyRegistry>,
}

impl AgentRegistry {
    // ... existing methods ...

    pub fn filter_by_basket(&self, basket: &str) -> Vec<Arc<Agent>> {
        let items = self.taxonomy.filter_by_basket(basket);
        items.iter()
            .filter_map(|item| {
                if item.item_type == ItemType::Agent {
                    self.agents.get(&item.id).cloned()
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn filter_by_sub_basket(&self, basket: &str, sub: &str) -> Vec<Arc<Agent>> {
        let items = self.taxonomy.filter_by_sub_basket(basket, sub);
        items.iter()
            .filter_map(|item| {
                if item.item_type == ItemType::Agent {
                    self.agents.get(&item.id).cloned()
                } else {
                    None
                }
            })
            .collect()
    }
}
```

#### Phase 5: CLI Commands

**Modify:**
- `frontend/rustic-ai-cli/src/repl.rs`

**Add commands:**
```rust
// /taxonomy list
// /taxonomy show <basket> [sub_basket]
// /taxonomy search <query>
```

**Estimated Effort:** 5-6 days

**Dependencies:**
- Gap 4 (Agent Registry Implementation) - AgentRegistry needs real methods

---

## Gap 3: Tool Coverage Gap (23+ Missing Tools)

### Current State

**Implemented tools (7):**
1. Shell (bash/PowerShell commands)
2. Filesystem (read, write, edit, glob, list, mkdir, delete, copy, move, info, hash)
3. HTTP (GET, POST, PUT, PATCH, DELETE, etc.)
4. SSH (connect, exec, disconnect, list_sessions, scp_upload, scp_download)
5. MCP (adapter for Model Context Protocol)
6. Skill (adapter for skill invocation)
7. Workflow (adapter for workflow execution)

### What's Missing (from `tools.md`)

#### Code Intelligence (1 tool)
- ❌ LSP (workspace/document symbol search)

#### Search Tools (4 tools)
- ❌ Grep (pattern search)
- ❌ Code search (semantic similarity + ranking)
- ❌ Web search
- ❌ Download (progress, resume, chunking)
- ❌ Crawler (html parsing, link extraction, robots)

#### Integration Tools (2 tools)
- ❌ Git (clone, pull, push, commit, status, diff, branches, tags)
- ❌ Database (sqlite, postgres, mysql connections)

#### Image Tools (1 tool)
- ❌ Image (resize, crop, rotate, convert, metadata)

#### Text Processing Tools (4 tools)
- ❌ Regex (multiline, groups, backrefs)
- ❌ Format (json/xml, minify)
- ❌ Encoding (base64, url, html entities, utf-8 validation)
- ❌ Convert (md<->html, csv<->json, xml<->json, yaml<->json)

#### Remote Tools (already partially covered)
- ✅ SSH exists
- ⚠️ Remote file ops (can use SSH for this)
- ⚠️ Host file ops (can use SCP via SSH)

**Total Missing: 12 tools (some tools combine multiple operations)**

### Why It Matters

From `REQUIREMENTS.md:58-108` (Use Cases):
- Programming/scripting in 20+ languages
- DevOps operations
- Database maintenance
- API development
- Cyber security
- Microsoft/Azure development
- AI/ML development
- Infrastructure as code
- Game development

Without these tools:
- Users can't interact with databases directly
- No git integration (core development workflow missing)
- No code intelligence (can't find symbols, navigate code)
- No semantic search (can't find relevant code sections)
- Limited web interaction (no search or downloads)
- Poor support for data workflows (no formatting/conversion)

### Implementation Plan

#### Priority 1: Essential Development Tools (Week 1-2)

**3.1 Git Tool**

**Create:** `rustic-ai-core/src/tools/git.rs`

```rust
pub struct GitTool {
    working_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "command")]
pub enum GitCommand {
    Clone {
        url: String,
        path: Option<String>,
        branch: Option<String>,
    },
    Pull {
        remote: Option<String>,
        branch: Option<String>,
    },
    Push {
        remote: Option<String>,
        branch: Option<String>,
    },
    Commit {
        message: String,
        add_patterns: Option<Vec<String>>,
        amend: bool,
    },
    Status {
        short: bool,
        branch: bool,
    },
    Diff {
        target: Option<String>,
        cached: bool,
        word_diff: bool,
    },
    Branch {
        create: Option<String>,
        delete: Option<String>,
        list: bool,
    },
    Tag {
        name: Option<String>,
        message: Option<String>,
        delete: Option<String>,
        list: bool,
    },
    Log {
        max_count: Option<usize>,
        format: Option<String>,
    },
    Checkout {
        branch: Option<String>,
        create_branch: bool,
    },
}
```

**Implementation:**
- Use `git2` crate for Rust bindings
- Stream output for long operations
- Respect working directory from context
- Add permission checks (read-only for status/diff/log, read-write for others)

**Estimated Effort:** 3 days

---

**3.2 Grep Tool**

**Create:** `rustic-ai-core/src/tools/grep.rs`

```rust
pub struct GrepTool {
    working_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrepArgs {
    /// Pattern to search for (supports regex)
    pub pattern: String,

    /// Directory to search (default: current)
    pub path: Option<String>,

    /// File pattern to match (*.rs, *.py, etc.)
    pub glob: Option<String>,

    /// Case insensitive search
    #[serde(default)]
    pub ignore_case: bool,

    /// Invert match
    #[serde(default)]
    pub invert_match: bool,

    /// Show line numbers
    #[serde(default)]
    pub line_numbers: bool,

    /// Show only filenames (no line content)
    #[serde(default)]
    pub files_only: bool,

    /// Max results to return (default: 100)
    pub max_results: Option<usize>,

    /// Context lines before/after match
    pub context: Option<usize>,
}

pub struct GrepMatch {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub line_content: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}
```

**Implementation:**
- Use `regex` crate for fast pattern matching
- Use `ignore` crate for efficient file walking with .gitignore support
- Stream results as they're found
- Limit results to prevent token bloat

**Estimated Effort:** 2 days

---

**3.3 Database Tool**

**Create:** `rustic-ai-core/src/tools/database.rs`

```rust
pub struct DatabaseTool {
    connections: HashMap<String, ConnectionPool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "command")]
pub enum DatabaseCommand {
    Connect {
        name: String,
        connection_string: String,
        db_type: DatabaseType,
    },
    Query {
        connection: String,
        sql: String,
        params: Option<Vec<Value>>,
    },
    ListTables {
        connection: String,
    },
    DescribeTable {
        connection: String,
        table: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseType {
    Sqlite,
    Postgres,
    Mysql,
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub rows_affected: Option<usize>,
    pub execution_time_ms: u64,
}
```

**Implementation:**
- `sqlx` for database abstraction
- SQLite support (file-based, already a dependency)
- Postgres support (add `sqlx-postgres` feature)
- MySQL support (add `sqlx-mysql` feature)
- Connection pooling per database
- Query streaming for large result sets
- Timeout and cancellation support

**Estimated Effort:** 4 days

---

#### Priority 2: Web & Search Tools (Week 3)

**3.4 Web Search Tool**

**Create:** `rustic-ai-core/src/tools/web_search.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct WebSearchArgs {
    /// Search query
    pub query: String,

    /// Number of results (default: 10, max: 50)
    pub num_results: Option<usize>,

    /// Search engine (default: auto)
    pub engine: Option<SearchEngine>,

    /// Language filter
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEngine {
    Google,
    Bing,
    DuckDuckGo,
    Auto,
}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub ranking: f32,
}
```

**Implementation:**
- Use `reqwest` for HTTP
- Search via APIs (DuckDuckGo has free API, Google requires key)
- Parse HTML responses
- Rank results by relevance
- Stream results as received

**Estimated Effort:** 2 days

---

**3.5 Download Tool**

**Create:** `rustic-ai-core/src/tools/download.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadArgs {
    /// URL to download
    pub url: String,

    /// Output path (default: filename from URL)
    pub output: Option<String>,

    /// Resume partial download
    #[serde(default)]
    pub resume: bool,

    /// Chunk size for streaming (default: 64KB)
    pub chunk_size: Option<usize>,

    /// Max size (default: 1GB)
    pub max_size: Option<usize>,

    /// Timeout in seconds (default: 300)
    pub timeout: Option<u64>,
}
```

**Implementation:**
- Use `reqwest` with streaming
- Support range requests for resume
- Progress reporting via events
- Size limits
- Timeout handling
- SHA256 hash verification

**Estimated Effort:** 2 days

---

#### Priority 3: Text & Data Tools (Week 4)

**3.6 Regex Tool**

**Create:** `rustic-ai-core/src/tools/regex.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct RegexArgs {
    /// Input text to process
    pub input: String,

    /// Regex pattern
    pub pattern: String,

    /// Operation type
    pub operation: RegexOperation,

    /// Case insensitive flag
    #[serde(default)]
    pub case_insensitive: bool,

    /// Multiline mode
    #[serde(default)]
    pub multiline: bool,

    /// Dot matches newline
    #[serde(default)]
    pub dot_all: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation")]
pub enum RegexOperation {
    Match {
        return_groups: bool,
    },
    Replace {
        replacement: String,
        global: bool,
    },
    FindAll {
        max_results: Option<usize>,
    },
}

pub struct RegexMatch {
    pub full_match: String,
    pub groups: Vec<Option<String>>,
    pub start: usize,
    pub end: usize,
}
```

**Implementation:**
- Use `regex` crate
- Support all flags
- Return structured matches with groups
- Replace with global option

**Estimated Effort:** 1 day

---

**3.7 Format Tool**

**Create:** `rustic-ai-core/src/tools/format.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct FormatArgs {
    /// Input data
    pub input: String,

    /// Operation
    pub operation: FormatOperation,

    /// Minify output
    #[serde(default)]
    pub minify: bool,

    /// Indent size (for formatting)
    pub indent: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op")]
pub enum FormatOperation {
    Json {
        pretty: bool,
    },
    Xml {
        pretty: bool,
    },
    XmlMinify,
    JsonMinify,
}
```

**Implementation:**
- `serde_json` for JSON
- `quick-xml` for XML
- Pretty printing with indent
- Minify mode

**Estimated Effort:** 1 day

---

**3.8 Encoding Tool**

**Create:** `rustic-ai-core/src/tools/encoding.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct EncodingArgs {
    /// Input data
    pub input: String,

    /// Operation
    pub operation: EncodingOperation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op")]
pub enum EncodingOperation {
    Base64Encode,
    Base64Decode,
    UrlEncode,
    UrlDecode,
    HtmlEntitiesEncode,
    HtmlEntitiesDecode,
    ValidateUtf8,
}

pub struct EncodingResult {
    pub output: String,
    pub is_valid: bool,
    pub errors: Vec<String>,
}
```

**Implementation:**
- `base64` crate
- `percent-encoding` crate
- `html-escape` crate
- UTF-8 validation

**Estimated Effort:** 1 day

---

**3.9 Convert Tool**

**Create:** `rustic-ai-core/src/tools/convert.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ConvertArgs {
    /// Input data
    pub input: String,

    /// Source format
    pub from: DataFormat,

    /// Target format
    pub to: DataFormat,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFormat {
    Json,
    Yaml,
    Xml,
    Csv,
    Html,
    Markdown,
}
```

**Implementation:**
- `serde_json` for JSON
- `serde_yaml` for YAML
- `quick-xml` for XML
- `csv` crate for CSV
- Markdown to HTML: `pulldown-cmark`
- HTML to Markdown: `html2md`

**Estimated Effort:** 2 days

---

#### Priority 4: Code Intelligence (Week 5)

**3.10 LSP Tool**

**Create:** `rustic-ai-core/src/tools/lsp.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct LspArgs {
    /// Workspace path
    pub workspace: String,

    /// Operation
    pub operation: LspOperation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op")]
pub enum LspOperation {
    SymbolSearch {
        query: String,
        kind: Option<SymbolKind>,
    },
    Definition {
        file: String,
        line: usize,
        column: usize,
    },
    References {
        file: String,
        line: usize,
        column: usize,
    },
    Hover {
        file: String,
        line: usize,
        column: usize,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Type,
    Variable,
    Constant,
    Module,
}
```

**Implementation:**
- Use `lsp-types` crate
- Communicate with LSP servers via stdio
- Start/stop LSP servers per language
- Cache symbols for workspace
- Provide search and navigation

**Estimated Effort:** 7 days (complex due to multiple language servers)

---

#### Priority 5: Image Tools (Week 6)

**3.11 Image Tool**

**Create:** `rustic-ai-core/src/tools/image.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ImageArgs {
    /// Input file path
    pub input: String,

    /// Operation
    pub operation: ImageOperation,

    /// Output file path (default: overwrite input)
    pub output: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op")]
pub enum ImageOperation {
    Resize {
        width: u32,
        height: u32,
        maintain_aspect: bool,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Rotate {
        degrees: u32,
    },
    Convert {
        format: ImageFormat,
    },
    Metadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
}

pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub size_bytes: usize,
}
```

**Implementation:**
- `image` crate for processing
- Support resize, crop, rotate, format conversion
- Extract metadata
- Stream progress for large images

**Estimated Effort:** 3 days

---

**Estimated Total Effort for All Tools:** 28-30 days (6 weeks)

**Dependencies:**
- None (can implement independently)

---

## Gap 4: Agent Registry Implementation

### Current State

**What exists:**
- `AgentRegistry` is defined as empty struct: `pub struct AgentRegistry;`
- Agents are stored in `HashMap<String, Arc<Agent>>` in `RusticAI` facade
- Basic get methods available through `RusticAI.get_agent(name)`

**What's missing:**
- No proper `AgentRegistry` implementation
- No filtering methods
- No query APIs
- Can't organize or manage agents programmatically

### Why It Matters

Without a proper registry:
- Can't implement taxonomy-based filtering (Gap 2)
- Can't filter agents by capability
- Can't implement agent discovery for sub-agent calling
- Poor scalability for large agent collections

### Implementation Plan

#### Create Full AgentRegistry

**Modify:** `rustic-ai-core/src/agents/registry.rs`

```rust
use std::collections::HashMap;
use std::sync::Arc;
use crate::agents::Agent;
use crate::config::schema::AgentConfig;

pub struct AgentRegistry {
    agents: HashMap<String, Arc<Agent>>,
    configs: HashMap<String, AgentConfig>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            configs: HashMap::new(),
        }
    }

    pub fn register(&mut self, config: AgentConfig, agent: Arc<Agent>) {
        let name = config.name.clone();
        self.agents.insert(name.clone(), agent);
        self.configs.insert(name, config);
    }

    pub fn get(&self, name: &str) -> Option<Arc<Agent>> {
        self.agents.get(name).cloned()
    }

    pub fn get_config(&self, name: &str) -> Option<&AgentConfig> {
        self.configs.get(name)
    }

    pub fn list_names(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    pub fn list_configs(&self) -> Vec<&AgentConfig> {
        self.configs.values().collect()
    }

    /// Filter agents by tool availability
    pub fn with_tool(&self, tool_name: &str) -> Vec<Arc<Agent>> {
        self.configs.values()
            .filter(|config| config.tools.iter().any(|t| t == tool_name))
            .filter_map(|config| self.get(&config.name))
            .collect()
    }

    /// Filter agents by skill availability
    pub fn with_skill(&self, skill_name: &str) -> Vec<Arc<Agent>> {
        self.configs.values()
            .filter(|config| config.skills.iter().any(|s| s == skill_name))
            .filter_map(|config| self.get(&config.name))
            .collect()
    }

    /// Filter agents by permission mode
    pub fn by_permission_mode(&self, mode: &AgentPermissionMode) -> Vec<Arc<Agent>> {
        self.configs.values()
            .filter(|config| config.permission_mode == *mode)
            .filter_map(|config| self.get(&config.name))
            .collect()
    }

    /// Find agents suitable for a task (heuristic)
    pub fn find_for_task(&self, task_description: &str) -> Vec<(Arc<Agent>, f32)> {
        // Simple heuristic based on tools/skills
        // Future: use embeddings/semantic search
        let mut results = Vec::new();
        for (name, config) in &self.configs {
            let score = Self::calculate_task_score(config, task_description);
            if let Some(agent) = self.get(name) {
                results.push((agent, score));
            }
        }
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        results
    }

    fn calculate_task_score(config: &AgentConfig, task: &str) -> f32 {
        // Very basic scoring based on description keywords
        let lower_task = task.to_lowercase();
        let mut score = 0.0f32;

        // Check tool names
        for tool in &config.tools {
            if lower_task.contains(&tool.to_lowercase()) {
                score += 1.0;
            }
        }

        // Check skills
        for skill in &config.skills {
            if lower_task.contains(&skill.to_lowercase()) {
                score += 1.5;
            }
        }

        score
    }
}
```

**Estimated Effort:** 2 days

**Dependencies:** None

---

## Gap 5: Dynamic Tool Loading (Load Only What's Needed)

### Current State

**What exists:**
- All tools registered at startup in `ToolManager`
- Each agent has a `tools: Vec<String>` whitelist
- ToolManager checks whitelist before executing

**What's missing:**
- All tool definitions loaded regardless of agent needs
- No lazy loading of tool instances
- No context-window-aware tool list management
- Potential token bloat from tool descriptions

### Why It Matters

From `big-picture.md:80-91`:
- "Context windows are managed explicitly (truncate/summarize/retain key messages)"
- "Share only necessary context between agents (filtered/summarized) to reduce token pressure"

Without dynamic loading:
- If you have 50 tools, all 50 descriptions go in system prompts
- Agent A uses 3 tools but gets descriptions for all 50
- Wasted tokens on irrelevant tools
- Worse with many agents (each agent sees all tools)

### Implementation Plan

#### Phase 1: Lazy Tool Registration

**Modify:** `rustic-ai-core/src/tools/manager.rs`

```rust
pub struct ToolManager {
    // Built-in tools always loaded
    built_in: HashMap<String, Arc<dyn Tool>>,

    // Lazy loaders: function to create tool on demand
    lazy_loaders: HashMap<String, Box<dyn Fn() -> Result<Arc<dyn Tool>> + Send + Sync>>,

    // Active tools: only tools that have been loaded
    active_tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolManager {
    pub fn new() -> Self {
        let built_in = HashMap::new();
        let lazy_loaders = HashMap::new();
        let active_tools = HashMap::new();

        // Register lazy loaders for heavy tools
        lazy_loaders.insert("lsp".to_string(), Box::new(|| {
            Ok(Arc::new(LspTool::new())?)
        }) as _);
        lazy_loaders.insert("database".to_string(), Box::new(|| {
            Ok(Arc::new(DatabaseTool::new())?)
        }) as _);
        lazy_loaders.insert("image".to_string(), Box::new(|| {
            Ok(Arc::new(ImageTool::new())?)
        }) as _);

        // Register built-in tools directly
        // ... shell, filesystem, http, ssh ...

        Self {
            built_in,
            lazy_loaders,
            active_tools,
        }
    }

    /// Get tool, loading lazily if needed
    pub async fn get_tool(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        // Check if already active
        if let Some(tool) = self.active_tools.get(name) {
            return Some(tool.clone());
        }

        // Check built-in
        if let Some(tool) = self.built_in.get(name) {
            let tool = tool.clone();
            self.active_tools.insert(name.to_string(), tool.clone());
            return Some(tool);
        }

        // Lazy load
        if let Some(loader) = self.lazy_loaders.get(name) {
            match loader() {
                Ok(tool) => {
                    self.active_tools.insert(name.to_string(), tool.clone());
                    return Some(tool);
                }
                Err(e) => {
                    tracing::error!("Failed to lazy load tool '{}': {}", name, e);
                    return None;
                }
            }
        }

        None
    }

    /// Get tool descriptions for an agent's whitelist
    pub fn get_tool_descriptions(&self, agent_tools: &[String]) -> Vec<ToolDescription> {
        agent_tools.iter()
            .filter_map(|name| self.describe(name))
            .collect()
    }

    /// Unload tools that haven't been used recently
    pub async fn unload_unused(&mut self, timeout_seconds: u64) {
        let now = std::time::Instant::now();
        // Keep built-in, unload lazy-loaded tools
        // Implementation depends on tracking usage timestamps
    }
}
```

#### Phase 2: Context-Aware Tool List

**Modify:** `rustic-ai-core/src/agents/behavior.rs`

```rust
impl Agent {
    fn system_prompt(&self, self, tool_manager: &ToolManager) -> String {
        let base_prompt = if let Some(ref template) = self.config.system_prompt_template {
            template.clone()
        } else {
            "You are a helpful AI assistant.".to_string()
        };

        if self.config.tools.is_empty() {
            return base_prompt;
        }

        // Get only tools this agent needs
        let tools = tool_manager.get_tool_descriptions(&self.config.tools);

        // Estimate token count of tool descriptions
        let tool_tokens = tools.iter()
            .map(|t| t.description.len() / 4)
            .sum::<usize>();

        let available_tokens = self.config.context_window_size
            .saturating_sub(self.system_prompt_len())
            .saturating_sub(self.estimated_history_tokens());

        // If tool descriptions exceed available tokens, truncate
        let tools_to_include = if tool_tokens > available_tokens {
            // Keep most important tools (priority ordering needed)
            tools.into_iter()
                .take_while(|t| {
                    // Simple heuristic: shorter descriptions first
                    t.description.len() < 500
                })
                .collect()
        } else {
            tools
        };

        let tools_json = serde_json::to_string(&tools_to_include).unwrap_or_default();
        format!(
            "{}\n\nWhen you need a tool, use these available tools:\n{}",
            base_prompt, tools_json
        )
    }
}
```

**Estimated Effort:** 3 days

**Dependencies:** None

---

## Gap 6: Agent-to-Agent Calling Protocol

### Current State

**What exists:**
- Workflow engine has `agent` step kind
- Can invoke agents via workflows
- Nested workflow support

**What's missing:**
- No direct agent-to-agent calling from within agent's act loop
- No sub-agent delegation protocol
- No context filtering between agents
- No mechanism to prevent duplicate context or window bloat

### Why It Matters

From `big-picture.md:99`:
- "Agents/sub-agents and multi-agent coordination (sequential + parallel)"

Without proper agent-to-agent calling:
- Agents can't dynamically delegate tasks to specialists
- No way to hand off to domain-specific agents
- Multi-agent workflows require pre-defined workflows (less flexible)
- Context duplication in multi-agent scenarios

### Implementation Plan (OpenCode-Style)

OpenCode approach:
- Agent A calls agent B with specific task
- Agent B receives filtered context (only what's relevant)
- Agent B's response is returned to Agent A
- Minimal context transfer to prevent bloat

#### Phase 1: Add Sub-Agent Call Tool

**Create:** `rustic-ai-core/src/tools/sub_agent.rs`

```rust
use crate::agents::Agent;
use crate::AgentRegistry;

pub struct SubAgentTool {
    registry: Arc<AgentRegistry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubAgentArgs {
    /// Target agent name
    pub agent_name: String,

    /// Task description for sub-agent
    pub task: String,

    /// Context to include (filtered, minimal)
    pub context: Option<ContextFilter>,

    /// Input data for the task
    pub input: Option<serde_json::Value>,

    /// Maximum tokens for sub-agent context (default: 4000)
    pub max_context_tokens: Option<usize>,

    /// Sub-agent tool whitelist (optional, uses agent's tools by default)
    pub allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextFilter {
    /// Include last N messages from current session
    pub last_messages: Option<usize>,

    /// Include specific context keys
    pub include_keys: Option<Vec<String>>,

    /// Include session summary if available
    pub include_summary: bool,

    /// Include current file/workspace context
    pub include_workspace: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubAgentResult {
    pub agent_name: String,
    pub success: bool,
    pub output: String,
    pub tool_calls: Vec<String>,
    pub tokens_used: usize,
}
```

#### Phase 2: Implement Filtered Context Building

**Modify:** `rustic-ai-core/src/agents/behavior.rs`

```rust
impl Agent {
    /// Build context for sub-agent call (filtered, minimal)
    async fn build_sub_agent_context(
        &self,
        session_id: uuid::Uuid,
        filter: Option<&ContextFilter>,
        max_tokens: usize,
    ) -> Result<Vec<ChatMessage>> {
        let system_prompt = self.system_prompt();

        // Get conversation history
        let messages = self.session_manager
            .get_session_messages(session_id)
            .await?;

        // Apply context filter
        let filtered = if let Some(filter) = filter {
            self.apply_context_filter(messages, filter, max_tokens).await?
        } else {
            // Default: include last 5 messages
            messages.into_iter()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        };

        let mut context = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
            name: None,
            tool_calls: None,
        }];

        // Estimate tokens and truncate if needed
        let mut token_count = system_prompt.len() / 4;
        for msg in filtered {
            let msg_tokens = msg.content.len() / 4;
            if token_count + msg_tokens > max_tokens {
                break;
            }
            token_count += msg_tokens;
            context.push(msg);
        }

        Ok(context)
    }

    async fn apply_context_filter(
        &self,
        messages: Vec<ChatMessage>,
        filter: &ContextFilter,
        max_tokens: usize,
    ) -> Result<Vec<ChatMessage>> {
        let mut result = Vec::new();

        // Include last N messages
        if let Some(n) = filter.last_messages {
            let take = messages.len().saturating_sub(n);
            result.extend(messages.into_iter().skip(take));
        } else {
            result.extend(messages);
        }

        // Include workspace context if requested
        if filter.include_workspace {
            let workspace_context = self.build_workspace_context().await?;
            result.push(ChatMessage {
                role: "system".to_string(),
                content: workspace_context,
                name: Some("workspace".to_string()),
                tool_calls: None,
            });
        }

        // Truncate to max tokens
        let mut token_count = 0;
        result = result.into_iter()
            .take_while(|msg| {
                let tokens = msg.content.len() / 4;
                if token_count + tokens <= max_tokens {
                    token_count += tokens;
                    true
                } else {
                    false
                }
            })
            .collect();

        Ok(result)
    }

    async fn build_workspace_context(&self) -> Result<String> {
        // Get current working directory
        let workdir = std::env::current_dir()?;

        // Build summary of workspace structure
        let mut context = format!("Working directory: {}\n\n", workdir.display());

        // List important files/directories
        if let Ok(entries) = std::fs::read_dir(&workdir) {
            context.push_str("Top-level entries:\n");
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    context.push_str(&format!("  [DIR] {}\n", path.display()));
                } else if path.is_file() {
                    context.push_str(&format!("  [FILE] {}\n", path.display()));
                }
            }
        }

        // Include project context if available
        if let Ok(project) = self.session_manager.get_project_info().await {
            context.push_str(&format!("\nProject: {}\nTech stack: {}\n",
                project.name,
                project.tech_stack.join(", ")
            ));
        }

        Ok(context)
    }
}
```

#### Phase 3: Sub-Agent Execution with Result Return

**Modify:** `rustic-ai-core/src/agents/coordinator.rs`

```rust
impl AgentCoordinator {
    /// Execute a sub-agent call from within an agent
    pub async fn call_sub_agent(
        &self,
        caller_agent_name: &str,
        args: SubAgentArgs,
        session_id: uuid::Uuid,
    ) -> Result<SubAgentResult> {
        // Get target agent
        let target_agent = self.registry.get(&args.agent_name)
            .ok_or_else(|| Error::NotFound(format!("Agent '{}' not found", args.agent_name)))?;

        // Build filtered context
        let caller = self.registry.get(caller_agent_name)
            .ok_or_else(|| Error::NotFound(format!("Caller agent '{}' not found", caller_agent_name)))?;

        let max_tokens = args.max_context_tokens.unwrap_or(4000);
        let context = caller.build_sub_agent_context(
            session_id,
            args.context.as_ref(),
            max_tokens,
        ).await?;

        // Execute sub-agent turn
        let start_time = std::time::Instant::now();

        // Create temporary session for sub-agent or share session
        let sub_session_id = session_id; // Share session for history continuity

        let mut messages = context;
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: args.task,
            name: Some(caller_agent_name.to_string()),
            tool_calls: None,
        });

        // Generate response
        let response = target_agent.provider.generate(&messages, &target_agent.generation_options()).await?;

        // Track tool calls if any
        let tool_calls = target_agent.extract_tool_calls(&response)
            .iter()
            .map(|c| c.tool.clone())
            .collect();

        // Store in shared session history
        self.session_manager.append_message(sub_session_id, "user", &args.task).await?;
        self.session_manager.append_message(sub_session_id, "assistant", &response).await?;

        // Calculate tokens used
        let duration = start_time.elapsed();
        let tokens_used = self.estimate_tokens(&messages) + self.estimate_tokens(&[ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
            name: None,
            tool_calls: None,
        }]);

        Ok(SubAgentResult {
            agent_name: args.agent_name,
            success: true,
            output: response,
            tool_calls,
            tokens_used,
        })
    }

    fn estimate_tokens(&self, messages: &[ChatMessage]) -> usize {
        messages.iter()
            .map(|m| m.content.len() / 4)
            .sum()
    }
}
```

#### Phase 4: Register Sub-Agent Tool

**Modify:** `rustic-ai-core/src/tools/manager.rs`

```rust
impl ToolManagerInit {
    pub async fn initialize(config: &ToolConfig, coordinator: Arc<AgentCoordinator>) -> Result<Arc<dyn Tool>> {
        match config.name.as_str() {
            "sub_agent" => {
                Ok(Arc::new(SubAgentTool::new(coordinator)))
            }
            // ... other tools ...
            _ => Err(Error::NotFound(format!("Unknown tool: {}", config.name))),
        }
    }
}
```

#### Phase 5: Add to Agent Default Tools

**Modify:** `rustic-ai-core/src/config/schema.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    // ... existing fields ...

    /// Allow this agent to call other agents
    #[serde(default)]
    pub allow_sub_agent_calls: bool,
}
```

#### Phase 6: Update CLI for Sub-Agent Calls

**Modify:** `frontend/rustic-ai-cli/src/repl.rs`

**Add:**
- Render sub-agent call events
- Show context filtering in action
- Display sub-agent results

**Estimated Effort:** 5 days

**Dependencies:**
- Gap 4 (Agent Registry Implementation)
- Gap 1 (Agent Permission Model - to enforce sub-agent permissions)

---

## Gap 7: Advanced Context Management

### Current State

**What exists:**
- Simple `AgentMemory` with `build_context_window()`
- LIFO (last-in-first-out) approach
- Rough token estimation (4 chars/token)
- Truncates when context window size hit

### What's Missing

From `REQUIREMENTS.md:132-139`:

#### 7.1 Context Pruning
- ❌ Remove redundant/less important messages
- ❌ Remove duplicate information
- ❌ Remove completed task ephemera

#### 7.2 Importance Scoring
- ❌ Score messages by importance
- ❌ Identify critical decisions
- ❌ Identify error patterns
- ❌ Keep high-importance messages longer

#### 7.3 Summarization
- ❌ Summarize older conversation
- ❌ Keep summaries in context
- ❌ Expand summaries when needed
- ❌ Provider-assisted summarization

#### 7.4 Context Compression
- ❌ Compress similar messages
- ❌ Extract key insights
- ❌ Represent context more efficiently

#### 7.5 Dynamic Management
- ❌ Adapt context based on current task
- ❌ Prune irrelevant context
- ❌ Add context based on task needs

### Why It Matters

From `REQUIREMENTS.md:132-139`:
- "Handle Limited Context Better"
- "Find important things from context bloat"
- "Identify rules and patterns"
- "Context pruning, compression, smart summarization, dynamic context management"

Without these:
- Large conversations hit context limits quickly
- Agent loses track of important decisions
- Can't maintain context for big codebases
- Poor handling of multi-turn tasks
- Inefficient token usage

### Implementation Plan

#### Phase 1: Message Importance Scoring

**Create:** `rustic-ai-core/src/agents/importance.rs`

```rust
use crate::providers::types::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportanceLevel {
    Critical,  // System prompts, explicit user instructions, decisions
    High,       // Code changes, major insights, error explanations
    Medium,     // Regular conversation, minor suggestions
    Low,         // Ephemeral updates, acknowledgments, progress
}

pub struct MessageScorer;

impl MessageScorer {
    pub fn score_message(&self, message: &ChatMessage) -> ImportanceLevel {
        // System message is always critical
        if message.role == "system" {
            return ImportanceLevel::Critical;
        }

        let content = message.content.to_lowercase();

        // Critical indicators
        if content.contains("decided") || content.contains("decision:")
            || content.contains("important:") || content.contains("critical:")
            || content.contains("never") || content.contains("always")
        {
            return ImportanceLevel::Critical;
        }

        // High indicators
        if content.contains("fixed") || content.contains("changed")
            || content.contains("implemented") || content.contains("created")
            || content.contains("error:") || content.contains("warning:")
        {
            return ImportanceLevel::High;
        }

        // Low indicators
        if content.len() < 50 || content == "ok"
            || content.starts_with("got it") || content.starts_with("thanks")
        {
            return ImportanceLevel::Low;
        }

        ImportanceLevel::Medium
    }

    pub fn score_messages(&self, messages: &[ChatMessage]) -> Vec<(usize, ImportanceLevel)> {
        messages.iter()
            .enumerate()
            .map(|(idx, msg)| (idx, self.score_message(msg)))
            .collect()
    }
}
```

#### Phase 2: Context Pruning

**Modify:** `rustic-ai-core/src/agents/memory.rs`

```rust
use crate::agents::importance::{MessageScorer, ImportanceLevel};

impl AgentMemory {
    /// Build context window with pruning
    pub async fn build_context_window_with_pruning(
        &self,
        messages: Vec<ChatMessage>,
        system_prompt: &str,
    ) -> Result<Vec<ChatMessage>> {
        let scorer = MessageScorer;
        let scored = scorer.score_messages(&messages);

        // Separate by importance
        let (critical, high, medium, low): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = scored.into_iter()
            .partition_map(|(idx, level)| {
                match level {
                    ImportanceLevel::Critical => Either::Left(idx),
                    ImportanceLevel::High => Either::Right(Left(idx)),
                    ImportanceLevel::Medium => Either::Right(Right(Left(idx))),
                    ImportanceLevel::Low => Either::Right(Right(Right(idx))),
                }
            });

        // Always keep critical and high importance
        let mut selected = critical.clone();
        selected.extend(high.clone());

        // Calculate remaining budget
        let system_tokens = system_prompt.len() / 4;
        let used_tokens: usize = selected.iter()
            .map(|&idx| messages.get(idx).map(|m| m.content.len()).unwrap_or(0) / 4)
            .sum();

        let remaining_tokens = self.context_window_size.saturating_sub(system_tokens + used_tokens);

        // Add medium importance until budget exhausted
        let mut token_count = used_tokens;
        for idx in medium {
            let msg_tokens = messages.get(idx).map(|m| m.content.len()).unwrap_or(0) / 4;
            if token_count + msg_tokens <= remaining_tokens {
                selected.push(idx);
                token_count += msg_tokens;
            } else {
                break;
            }
        }

        // Add low importance only if space remains
        for idx in low {
            let msg_tokens = messages.get(idx).map(|m| m.content.len()).unwrap_or(0) / 4;
            if token_count + msg_tokens <= remaining_tokens {
                selected.push(idx);
                token_count += msg_tokens;
            } else {
                break;
            }
        }

        // Sort by original order and build context
        selected.sort();
        let mut context = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
            name: None,
            tool_calls: None,
        }];

        context.extend(selected.into_iter()
            .filter_map(|idx| messages.get(idx).cloned()));

        Ok(context)
    }
}
```

#### Phase 3: Provider-Assisted Summarization

**Modify:** `rustic-ai-core/src/agents/memory.rs`

```rust
use crate::providers::types::ModelProvider;

pub struct AgentMemory {
    // ... existing fields ...
    summaries: Vec<ConversationSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub turn_range: (usize, usize),  // Original indices
    pub summary: String,
    pub key_decisions: Vec<String>,
    pub key_errors: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl AgentMemory {
    /// Build context with summarization
    pub async fn build_context_window_with_summaries(
        &self,
        messages: Vec<ChatMessage>,
        system_prompt: &str,
        provider: &dyn ModelProvider,
    ) -> Result<Vec<ChatMessage>> {
        let system_tokens = system_prompt.len() / 4;
        let available_tokens = self.context_window_size.saturating_sub(system_tokens);

        // Try to fit all messages
        let mut context = Vec::new();
        let mut token_count = 0usize;

        for (idx, msg) in messages.iter().enumerate().rev() {
            let msg_tokens = msg.content.len() / 4;

            // Check if current message + existing fits
            if token_count + msg_tokens <= available_tokens {
                context.insert(0, msg.clone());
                token_count += msg_tokens;
            } else {
                // Doesn't fit - need to summarize older messages
                let older_msgs = &messages[..idx];
                if !older_msgs.is_empty() {
                    let summary = self.summarize_messages(older_msgs, provider).await?;
                    // Add summary to context
                    context.insert(0, ChatMessage {
                        role: "system".to_string(),
                        content: format!("[Summary of previous conversation: {}]", summary),
                        name: Some("summary".to_string()),
                        tool_calls: None,
                    });
                }
                break;
            }
        }

        context.insert(0, ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
            name: None,
            tool_calls: None,
        });

        Ok(context)
    }

    async fn summarize_messages(
        &self,
        messages: &[ChatMessage],
        provider: &dyn ModelProvider,
    ) -> Result<String> {
        let conversation = messages.iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let summary_prompt = format!(
            "Summarize the following conversation. Extract key decisions and errors. \
             Keep it concise (max 200 words):\n\n{}",
            conversation
        );

        let summary = provider.generate(&[
            ChatMessage {
                role: "user".to_string(),
                content: summary_prompt,
                name: None,
                tool_calls: None,
            }
        ], &GenerateOptions::default()).await?;

        Ok(summary)
    }
}
```

#### Phase 4: Duplicate Detection

**Create:** `rustic-ai-core/src/agents/dedup.rs`

```rust
use crate::providers::types::ChatMessage;

pub struct ContextDeduplicator;

impl ContextDeduplicator {
    /// Remove duplicate or near-duplicate messages
    pub fn dedup_messages(&self, messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        if messages.len() < 2 {
            return messages;
        }

        let mut deduped = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for msg in messages {
            // Normalize content for comparison
            let normalized = self.normalize_content(&msg.content);

            // Skip if we've seen this before
            if seen.contains(&normalized) {
                continue;
            }

            // Mark as seen
            seen.insert(normalized);
            deduped.push(msg);
        }

        deduped
    }

    fn normalize_content(&self, content: &str) -> String {
        content.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
    }
}
```

#### Phase 5: Dynamic Context Management

**Modify:** `rustic-ai-core/src/agents/behavior.rs`

```rust
impl Agent {
    /// Analyze current task and adjust context dynamically
    async fn optimize_context_for_task(
        &self,
        messages: Vec<ChatMessage>,
        current_task: &str,
    ) -> Result<Vec<ChatMessage>> {
        // Extract keywords from task
        let keywords = self.extract_keywords(current_task);

        // Score messages by relevance to task
        let scored: Vec<(usize, f32)> = messages.iter()
            .enumerate()
            .map(|(idx, msg)| {
                let relevance = self.calculate_relevance(msg, &keywords);
                (idx, relevance)
            })
            .collect();

        // Keep most relevant messages
        let mut selected: Vec<_> = scored.into_iter()
            .filter(|&(_, relevance)| relevance > 0.3)  // Threshold
            .collect();

        // Sort by relevance (descending)
        selected.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Take top messages that fit in context
        let mut context = Vec::new();
        let mut token_count = 0usize;

        for (idx, _) in selected {
            if let Some(msg) = messages.get(idx) {
                let msg_tokens = msg.content.len() / 4;
                if token_count + msg_tokens <= self.config.context_window_size / 2 {
                    context.push(msg.clone());
                    token_count += msg_tokens;
                } else {
                    break;
                }
            }
        }

        // Always include system prompt
        let system_prompt = self.system_prompt();
        context.insert(0, ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
            name: None,
            tool_calls: None,
        });

        Ok(context)
    }

    fn extract_keywords(&self, task: &str) -> Vec<String> {
        // Simple keyword extraction
        task.split_whitespace()
            .filter(|w| w.len() > 3)  // Ignore short words
            .map(|w| w.to_lowercase())
            .collect()
    }

    fn calculate_relevance(&self, message: &ChatMessage, keywords: &[String]) -> Vec<ChatMessage> {
        let content = message.content.to_lowercase();

        let matches = keywords.iter()
            .filter(|kw| content.contains(kw))
            .count();

        let max_matches = keywords.len();
        if max_matches == 0 {
            return 0.5;  // Default relevance
        }

        matches as f32 / max_matches as f32
    }
}
```

**Estimated Effort:** 8-10 days

**Dependencies:**
- Provider integration (for summarization)

---

## Gap 8: Big Codebase Support (RAG, Vector DB, Indexing)

### Current State

**What exists:**
- Simple filesystem tool for reading files
- Grep not yet implemented (Gap 3)
- No code indexing
- No vector database
- No semantic search

### What's Missing

From `REQUIREMENTS.md:123-130`:

#### 8.1 Code Indexing
- ❌ Parse code structure (AST)
- ❌ Extract symbols (functions, classes, types)
- ❌ Build call graphs
- ❌ Track dependencies
- ❌ Store index persistently

#### 8.2 Vector Database
- ❌ Store embeddings for files, functions, code blocks
- ❌ Vector similarity search
- ❌ Support for different embedding providers

#### 8.3 RAG (Retrieval Augmented Generation)
- ❌ Retrieve relevant code sections
- ❌ Augment agent prompts with retrieved context
- ❌ Re-ranking by relevance

#### 8.4 Semantic Search
- ❌ Find code by semantic similarity
- ❌ Natural language queries over codebase
- ❌ Cross-language code understanding

### Why It Matters

From `REQUIREMENTS.md:123-130`:
- "Handle Big Codebases Better"
- "100K+ files"
- "Semantic search in < 100ms"

Without these:
- Agents can't understand large codebases
- Can't find relevant code sections
- Can't answer questions about architecture
- Poor navigation in large projects
- Can't handle complex refactors

### Implementation Plan

#### Phase 1: Code Indexing Engine

**Create module:** `rustic-ai-core/src/indexing/`

**Files:**
- `indexing/mod.rs`
- `indexing/parser.rs`
- `indexing/symbols.rs`
- `indexing/types.rs`

`indexing/types.rs`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIndex {
    pub workspace: PathBuf,
    pub files: Vec<FileIndex>,
    pub symbols: Vec<SymbolIndex>,
    pub dependencies: Vec<DependencyIndex>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndex {
    pub path: PathBuf,
    pub language: String,
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    pub imports: Vec<ImportInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolIndex {
    pub name: String,
    pub symbol_type: SymbolType,
    pub file_path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub docstring: Option<String>,
    pub signature: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolType {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Type,
    Variable,
    Constant,
    Module,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub line: usize,
    pub params: Vec<ParamInfo>,
    pub return_type: Option<String>,
    pub docstring: Option<String>,
}
```

**Implementation:**
- Use `tree-sitter` for parsing multiple languages
- Support: Rust, Python, JavaScript/TypeScript, Go, C/C++
- Extract AST nodes for symbols
- Build call graph
- Store in SQLite (schema extension)

**Estimated Effort:** 10-12 days

---

#### Phase 2: Vector Database Integration

**Create module:** `rustic-ai-core/src/vector/`

**Files:**
- `vector/mod.rs`
- `vector/db.rs`
- `vector/embedding.rs`

`vector/types.rs`:
```rust
pub struct VectorDB {
    backend: VectorBackend,
    embedding_provider: Arc<dyn EmbeddingProvider>,
}

pub enum VectorBackend {
    SqliteVector,  // Use sqlite with vector extension
    Qdrant,
    Weaviate,
    Milvus,
    PostgresVector,  // pgvector
}

pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn dimension(&self) -> usize;
}

pub struct Embedding {
    pub id: String,
    pub vector: Vec<f32>,
    pub metadata: serde_json::Value,
}

pub struct SearchQuery {
    pub text: String,
    pub top_k: usize,
    pub filter: Option<serde_json::Value>,
}

pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub metadata: serde_json::Value,
}
```

**Implementation:**
1. Start with SQLite + vector-sqlite extension
2. Support embedding providers:
   - OpenAI (text-embedding-3-small/large)
   - OpenAI-compatible (local)
   - Sentence-transformers (local Python)
3. Index code blocks and symbols
4. Implement similarity search (cosine similarity)
5. Store metadata (file path, symbol, context)

**Estimated Effort:** 7-10 days

---

#### Phase 3: RAG System

**Create module:** `rustic-ai-core/src/rag/`

**Files:**
- `rag/mod.rs`
- `rag/retriever.rs`
- `rag/augmenter.rs`

`rag/types.rs`:
```rust
pub struct RAGRetriever {
    index: Arc<CodeIndex>,
    vector_db: Arc<VectorDB>,
}

#[derive(Debug, Clone)]
pub struct RetrievalRequest {
    pub query: String,
    pub top_k: usize,
    pub min_score: f32,
    pub file_filter: Option<Vec<String>>,
    pub symbol_filter: Option<Vec<SymbolType>>,
}

pub struct RetrievalResult {
    pub snippets: Vec<CodeSnippet>,
    pub symbols: Vec<SymbolMatch>,
}

#[derive(Debug, Clone)]
pub struct CodeSnippet {
    pub file_path: PathBuf,
    pub content: String,
    pub line_start: usize,
    pub line_end: usize,
    pub score: f32,
    pub context_before: String,
    pub context_after: String,
}

#[derive(Debug, Clone)]
pub struct SymbolMatch {
    pub symbol: SymbolIndex,
    pub score: f32,
    pub usage_context: Option<String>,
}
```

**Implementation:**
1. Hybrid search:
   - Keyword search (exact matches)
   - Semantic search (vector similarity)
   - Combine and re-rank results
2. Context expansion:
   - Include surrounding lines
   - Include function/class context
3. Ranking:
   - Score by relevance
   - Score by recency
   - Score by importance
4. Augmentation:
   - Inject retrieved context into agent prompt
   - Format for readability
   - Include source references

**Estimated Effort:** 6-8 days

---

#### Phase 4: Integration with Agent

**Modify:** `rustic-ai-core/src/agents/behavior.rs`

```rust
impl Agent {
    async fn build_context_window_with_rag(
        &self,
        messages: Vec<ChatMessage>,
        system_prompt: &str,
        retriever: Arc<RAGRetriever>,
    ) -> Result<Vec<ChatMessage>> {
        // Build base context
        let mut context = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
            name: None,
            tool_calls: None,
        }];

        // Get last user query
        let last_query = messages.iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // Retrieve relevant code
        let retrieval = retriever.retrieve(RetrievalRequest {
            query: last_query.clone(),
            top_k: 5,
            min_score: 0.7,
            file_filter: None,
            symbol_filter: None,
        }).await?;

        // Build code context from retrieval
        let mut code_context = String::new();
        for snippet in &retrieval.snippets {
            code_context.push_str(&format!(
                "File: {} (lines {}-{})\n{}\n\n",
                snippet.file_path.display(),
                snippet.line_start,
                snippet.line_end,
                snippet.content
            ));
        }

        // Add retrieved context as system message
        context.push(ChatMessage {
            role: "system".to_string(),
            content: format!("Relevant code from repository:\n{}", code_context),
            name: Some("rag_context".to_string()),
            tool_calls: None,
        });

        // Add conversation history (limited)
        let history_tokens = self.config.context_window_size
            .saturating_sub((system_prompt.len() + code_context.len()) / 4);

        let mut token_count = 0usize;
        for msg in messages.iter().rev() {
            let msg_tokens = msg.content.len() / 4;
            if token_count + msg_tokens <= history_tokens {
                context.insert(1, msg.clone());  // After system and RAG context
                token_count += msg_tokens;
            } else {
                break;
            }
        }

        // Reverse to correct order
        context[2..].reverse();

        Ok(context)
    }
}
```

**Estimated Effort:** 3 days

---

**Total Effort for Big Codebase Support:** 26-30 days (5-6 weeks)

**Dependencies:**
- None (can implement independently)

---

## Gap 9: Self-Learning System

### Current State

**What exists:**
- Session-based permission decisions (`AllowInSession` remembered)
- Manual runtime overrides persisted to config
- No explicit learning or feedback system

### What's Missing

From `REQUIREMENTS.md:159-168`:

#### 9.1 Learn from Mistakes
- ❌ Track common errors
- ❌ Identify patterns in failures
- ❌ Suggest alternative approaches
- ❌ Avoid repeating mistakes

#### 9.2 Learn from User Feedback
- ❌ Collect explicit feedback (thumbs up/down)
- ❌ Learn from corrections
- ❌ Adapt to user preferences
- ❌ Update behavior based on feedback

#### 9.3 Learn from User Frustrations
- ❌ Detect frustration signals (rephrasing, cancellations)
- ❌ Identify problematic patterns
- ❌ Adjust approach to reduce frustration

#### 9.4 Success Pattern Library
- ❌ Store successful patterns
- ❌ Reuse working solutions
- ❌ Build knowledge base of what works

#### 9.5 Continuous Improvement
- ❌ Learn from each session
- ❌ Update models over time
- ❌ Track metrics and trends

### Why It Matters

From `REQUIREMENTS.md:159-168`:
- "Don't make same mistakes over and over again"
- "Learn from user feedback"
- "Continuous improvement loops"

Without these:
- Agents repeat same mistakes
- Don't adapt to user's working style
- Can't improve over time
- Poor long-term user experience

### Implementation Plan

#### Phase 1: Feedback Collection System

**Create module:** `rustic-ai-core/src/learning/`

**Files:**
- `learning/mod.rs`
- `learning/feedback.rs`
- `learning/storage.rs`

`learning/types.rs`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFeedback {
    pub id: String,
    pub session_id: String,
    pub agent_name: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub feedback_type: FeedbackType,
    pub rating: i8,  // -1 to 1 (negative to positive)
    pub comment: Option<String>,
    pub context: FeedbackContext,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackType {
    ToolExecution,
    CodeSuggestion,
    ResponseQuality,
    ContextSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackContext {
    pub task_description: String,
    pub tools_used: Vec<String>,
    pub model_response: String,
    pub error_occurred: bool,
    pub error_message: Option<String>,
}
```

**Implementation:**
1. Add feedback collection command to CLI
2. Store feedback in SQLite (new table)
3. Provide API for agents to collect feedback implicitly

**Estimated Effort:** 3 days

---

#### Phase 2: Mistake Pattern Learning

**Create:** `rustic-ai-core/src/learning/patterns.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MistakePattern {
    pub id: String,
    pub pattern_type: MistakeType,
    pub trigger: String,
    pub frequency: usize,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MistakeType {
    PermissionDenied,
    ToolTimeout,
    FileNotFound,
    CompilationError,
    TestFailure,
    WrongApproach,  // Detected from negative feedback
}

pub struct PatternLearner {
    patterns: Vec<MistakePattern>,
}

impl PatternLearner {
    pub fn record_event(&mut self, event: &Event) {
        match event {
            Event::ToolFailed { tool, error } => {
                self.record_failure(tool, error);
            }
            Event::PermissionDenied { tool } => {
                self.record_permission_denied(tool);
            }
            _ => {}
        }
    }

    fn record_failure(&mut self, tool: &str, error: &str) {
        // Analyze error to identify pattern type
        let pattern_type = self.classify_error(error);

        // Find or create pattern
        if let Some(pattern) = self.patterns.iter_mut().find(|p| p.trigger == tool && p.pattern_type == pattern_type) {
            pattern.frequency += 1;
            pattern.last_seen = Utc::now();
        } else {
            self.patterns.push(MistakePattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type,
                trigger: tool.to_string(),
                frequency: 1,
                last_seen: Utc::now(),
                suggested_fix: self.suggest_fix(pattern_type, error),
            });
        }
    }

    fn classify_error(&self, error: &str) -> MistakeType {
        let lower = error.to_lowercase();

        if lower.contains("permission denied") || lower.contains("access denied") {
            return MistakeType::PermissionDenied;
        }
        if lower.contains("timeout") || lower.contains("timed out") {
            return MistakeType::ToolTimeout;
        }
        if lower.contains("not found") || lower.contains("no such file") {
            return MistakeType::FileNotFound;
        }
        if lower.contains("compilation error") || lower.contains("syntax error") {
            return MistakeType::CompilationError;
        }
        if lower.contains("test failed") || lower.contains("assertion failed") {
            return MistakeType::TestFailure;
        }

        MistakeType::WrongApproach
    }

    fn suggest_fix(&self, pattern_type: MistakeType, error: &str) -> Option<String> {
        match pattern_type {
            MistakeType::PermissionDenied => {
                Some("Check file permissions or request permission with higher scope".to_string())
            }
            MistakeType::ToolTimeout => {
                Some("Increase timeout or break task into smaller steps".to_string())
            }
            MistakeType::FileNotFound => {
                Some("Check file path and working directory".to_string())
            }
            _ => None,
        }
    }

    pub fn get_active_patterns(&self) -> Vec<&MistakePattern> {
        self.patterns.iter()
            .filter(|p| p.frequency >= 3)  // Only show patterns seen 3+ times
            .collect()
    }

    pub fn get_pattern_suggestion(&self, tool: &str) -> Option<&MistakePattern> {
        self.patterns.iter()
            .filter(|p| p.trigger == tool)
            .max_by_key(|p| p.frequency)
    }
}
```

**Estimated Effort:** 4 days

---

#### Phase 3: Preference Adaptation

**Create:** `rustic-ai-core/src/learning/preferences.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    pub id: String,
    pub session_id: String,
    pub preferences: HashMap<String, PreferenceValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PreferenceValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

pub struct PreferenceLearner {
    preferences: HashMap<String, PreferenceValue>,
}

impl PreferenceLearner {
    pub fn record_choice(&mut self, context: &str, choice: &str) {
        let key = format!("choice_{}", context);
        self.preferences.insert(key, PreferenceValue::String(choice.to_string()));
    }

    pub fn record_rating(&mut self, context: &str, rating: i8) {
        let key = format!("rating_{}", context);
        self.preferences.insert(key, PreferenceValue::Int(rating as i64));
    }

    pub fn get_preference(&self, context: &str) -> Option<&PreferenceValue> {
        let key = format!("choice_{}", context);
        self.preferences.get(&key)
    }

    pub fn get_preferred_approach(&self, task_type: &str) -> Option<String> {
        let key = format!("approach_{}", task_type);
        self.preferences.get(&key).and_then(|v| {
            if let PreferenceValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
    }
}
```

**Estimated Effort:** 2 days

---

#### Phase 4: Success Pattern Library

**Create:** `rustic-ai-core/src/learning/success_patterns.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessPattern {
    pub id: String,
    pub name: String,
    pub category: PatternCategory,
    pub description: String,
    pub template: String,
    pub frequency: usize,
    pub last_used: chrono::DateTime<chrono::Utc>,
    pub success_rate: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternCategory {
    ErrorFixing,
    Refactoring,
    Debugging,
    FeatureImplementation,
    Testing,
}

pub struct SuccessPatternLibrary {
    patterns: Vec<SuccessPattern>,
}

impl SuccessPatternLibrary {
    pub fn record_success(&mut self, category: PatternCategory, description: &str) {
        // Find similar pattern
        if let Some(pattern) = self.patterns.iter_mut().find(|p| {
            p.category == category &&
            self.similarity(&p.description, description) > 0.8
        }) {
            pattern.frequency += 1;
            pattern.last_used = Utc::now();
        } else {
            self.patterns.push(SuccessPattern {
                id: uuid::Uuid::new_v4().to_string(),
                name: Self::generate_name(category, description),
                category,
                description: description.to_string(),
                template: Self::extract_template(description),
                frequency: 1,
                last_used: Utc::now(),
                success_rate: 1.0,
            });
        }
    }

    pub fn find_patterns(&self, task: &str, category: Option<PatternCategory>) -> Vec<&SuccessPattern> {
        self.patterns.iter()
            .filter(|p| {
                category.map_or(true, |c| p.category == c) &&
                self.similarity(&p.description, task) > 0.5
            })
            .collect()
    }

    pub fn get_top_patterns(&self, category: PatternCategory, limit: usize) -> Vec<&SuccessPattern> {
        let mut filtered: Vec<_> = self.patterns.iter()
            .filter(|p| p.category == category)
            .collect();

        // Sort by success_rate * frequency
        filtered.sort_by(|a, b| {
            let score_a = a.success_rate * a.frequency as f32;
            let score_b = b.success_rate * b.frequency as f32;
            score_b.partial_cmp(&score_a).unwrap()
        });

        filtered.into_iter().take(limit).collect()
    }

    fn similarity(&self, a: &str, b: &str) -> f32 {
        // Simple word overlap similarity
        let set_a: HashSet<_> = a.split_whitespace().collect();
        let set_b: HashSet<_> = b.split_whitespace().collect();

        let intersection = set_a.intersection(&set_b).count();
        let union = set_a.union(&set_b).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f32 / union as f32
    }
}
```

**Estimated Effort:** 3 days

---

#### Phase 5: Integration with Agent

**Modify:** `rustic-ai-core/src/agents/behavior.rs`

```rust
impl Agent {
    pub async fn run_with_learning(
        &self,
        session_id: uuid::Uuid,
        event_tx: mpsc::Sender<Event>,
        pattern_learner: Arc<PatternLearner>,
        preference_learner: Arc<PreferenceLearner>,
        success_library: Arc<SuccessPatternLibrary>,
    ) -> Result<()> {
        // Check for known patterns before starting
        if let Some(pattern) = pattern_learner.get_pattern_suggestion(&self.config.name) {
            let _ = event_tx.try_send(Event::Progress(format!(
                "Note: This task has a known pattern that might fail: {}",
                pattern.suggested_fix.as_deref().unwrap_or("Try a different approach")
            )));
        }

        // Run normal agent loop
        let result = self.run_assistant_tool_loop(session_id, /* ... */).await;

        // Record result for learning
        match result {
            Ok(_) => {
                // Task succeeded - record success pattern
                let task_desc = self.get_current_task_description().await?;
                success_library.record_success(PatternCategory::FeatureImplementation, &task_desc);
            }
            Err(e) => {
                // Task failed - learn from it
                pattern_learner.record_event(&Event::Error(e.to_string()));
            }
        }

        result
    }
}
```

**Estimated Effort:** 2 days

---

**Total Effort for Self-Learning System:** 14-18 days (3-4 weeks)

**Dependencies:**
- None (can implement independently)

---

## Summary of Implementation Effort

| Gap | Priority | Estimated Effort | Dependencies |
|------|-----------|------------------|--------------|
| 1. Agent Permission Model (Read/Write) | High | 4-5 days | None |
| 2. Taxonomy Implementation | Medium | 5-6 days | Gap 4 |
| 3. Tool Coverage Gap | High | 28-30 days (6 weeks) | None |
| 4. Agent Registry Implementation | High | 2 days | None |
| 5. Dynamic Tool Loading | Medium | 3 days | None |
| 6. Agent-to-Agent Calling | High | 5 days | Gaps 1, 4 |
| 7. Advanced Context Management | High | 8-10 days | None |
| 8. Big Codebase Support (RAG, Vector DB) | Critical | 26-30 days (5-6 weeks) | None |
| 9. Self-Learning System | Medium | 14-18 days (3-4 weeks) | None |

**Total Estimated Effort: 95-110 days (19-22 weeks)**

---

## Recommended Implementation Order

### Phase 1: Foundation (Weeks 1-3)
1. **Gap 4: Agent Registry** (2 days) - Quick win, enables others
2. **Gap 1: Agent Permission Model** (4-5 days) - Enables read-only agents
3. **Gap 6: Agent-to-Agent Calling** (5 days) - Depends on 1 and 4

### Phase 2: Organization & Context (Weeks 4-6)
4. **Gap 2: Taxonomy Implementation** (5-6 days) - Better organization
5. **Gap 7: Advanced Context Management** (8-10 days) - Better context usage
6. **Gap 5: Dynamic Tool Loading** (3 days) - Reduce token bloat

### Phase 3: Tools Expansion (Weeks 7-12)
7. **Gap 3: Tool Coverage** (28-30 days) - Implement 12 missing tools
   - Priority 1: Git, Grep, Database (Week 7-8)
   - Priority 2: Web Search, Download (Week 9)
   - Priority 3: Regex, Format, Encoding, Convert (Week 10)
   - Priority 4: LSP (Week 11)
   - Priority 5: Image (Week 12)

### Phase 4: Advanced Features (Weeks 13-19)
8. **Gap 9: Self-Learning System** (14-18 days) - Continuous improvement
   - Phase 1: Feedback Collection (Week 13)
   - Phase 2: Mistake Patterns (Week 14-15)
   - Phase 3: Preference Adaptation (Week 16)
   - Phase 4: Success Patterns (Week 17)
   - Phase 5: Integration (Week 18)

### Phase 5: Big Codebase Support (Weeks 20-22)
9. **Gap 8: Big Codebase Support** (26-30 days) - Critical for large projects
   - Phase 1: Code Indexing (Week 20-22)

---

## Integration Points

These implementations must integrate with existing systems:

1. **Config System**
   - Add new config fields to `config/schema.rs`
   - Update `config.schema.json` with new properties
   - Update validation

2. **Storage System**
   - Add new tables to SQLite schema
   - Create migration scripts
   - Implement storage backend methods

3. **Event System**
   - Emit new event types for learning feedback
   - Emit events for agent-to-agent calls
   - Emit events for pattern detection

4. **CLI/UI**
   - Add commands for:
     - `/agents list --basket <name>` (Gap 2)
     - `/agents filter --permission-mode <mode>` (Gap 1)
     - `/feedback --type <type>` (Gap 9)
     - `/patterns list` (Gap 9)
     - `/rag search <query>` (Gap 8)
   - Render new event types
   - Display learning insights

5. **Testing**
   - Add tests for each new feature
   - Integration tests for agent-to-agent calling
   - Performance tests for vector DB search
   - User studies for learning effectiveness

---

## Success Criteria

Each gap is considered complete when:

1. **Agent Permission Model**
   - ✅ `AgentPermissionMode` enum exists and is used
   - ✅ Tools respect read-only vs read-write
   - ✅ Config supports permission mode per agent
   - ✅ CLI displays agent permission modes

2. **Taxonomy Implementation**
   - ✅ `TaxonomyRegistry` is functional
   - ✅ Agents/tools/skills have taxonomy_membership
   - ✅ Filtering APIs work correctly
   - ✅ CLI commands for taxonomy browsing

3. **Tool Coverage**
   - ✅ All 12 missing tools implemented
   - ✅ Each tool has tests
   - ✅ Tools are registered and discoverable
   - ✅ Documentation updated for each tool

4. **Agent Registry**
   - ✅ `AgentRegistry` has query methods
   - ✅ Filtering by tool/skill/permission works
   - ✅ Task suitability heuristics implemented

5. **Dynamic Tool Loading**
   - ✅ Lazy loading works for heavy tools
   - ✅ ToolManager tracks active tools
   - ✅ Context-aware tool descriptions
   - ✅ Tool descriptions fit in context window

6. **Agent-to-Agent Calling**
   - ✅ `SubAgentTool` works correctly
   - ✅ Context filtering prevents bloat
   - ✅ Sub-agent results returned properly
   - ✅ No duplicate context in shared sessions

7. **Advanced Context Management**
   - ✅ Importance scoring identifies critical messages
   - ✅ Pruning removes duplicates
   - ✅ Summarization compresses old conversation
   - ✅ Dynamic adaptation adjusts context per task

8. **Big Codebase Support**
   - ✅ Code indexing parses major languages
   - ✅ Vector DB stores embeddings
   - ✅ Semantic search returns relevant code
   - ✅ RAG augments agent prompts with context

9. **Self-Learning System**
   - ✅ User feedback is collected
   - ✅ Mistake patterns are detected
   - ✅ Preferences are learned
   - ✅ Success patterns are recorded
   - ✅ Agent adapts behavior based on learning

---

## Next Steps

1. **Prioritize based on impact and dependencies:**
   - Start with Phase 1 (Foundation)
   - Follow with Phase 2 (Organization & Context)
   - Then Phase 3 (Tools) - can work in parallel
   - Then Phase 4-5 (Advanced)

2. **Create TODO.md entries** for each gap:
   - Break down into sub-tasks
   - Set priorities
   - Track progress

3. **Update design docs:**
   - Add ADRs for major decisions
   - Update `DESIGN_GUIDE.md` with new patterns
   - Update integration plan

4. **Start implementation:**
   - Begin with highest priority, lowest effort
   - Follow the quality gate workflow
   - Update documentation as you go

---

**End of Document**
