# Implementation Plan: Four Optional Features

**Status:** ✅ IMPLEMENTED (Phases A-E completed)
**Date:** 2026-02-12
**Items:** TODO.md items 25, 26, 28, 29

---

## Overview

This plan implements four optional, fully configurable features for Rustic-AI production readiness:

1. **Feature A:** Aggressive Context Summary (Hybrid trigger, quality tracking)
2. **Feature B:** TODO Tracking (Session + project scopes, hierarchy, metadata)
3. **Feature C:** Sub-Agent Orchestration v2 (Parallel, hybrid caching, visibility, auto-TODOs)
4. **Feature D:** Dynamic Routing (LangGraph-like, fallback, integration with all above)

All features:
- Fully optional via `FeatureConfig` toggles
- Per-agent config overrides where applicable
- Storage backends: SQLite + Postgres
- CLI diagnostics and visibility
- Events for tracing and debugging

**Estimated timeline:** 7 weeks (including cross-feature integration)

---

## FEATURE A: Aggressive Context Summary (3-4 days)

### A1. Config Schema

**Modify:** `rustic-ai-core/src/config/schema.rs`

```rust
pub struct FeatureConfig {
    // ... existing ...
    pub aggressive_summary_enabled: bool,
}

pub struct SummarizationConfig {
    pub enabled: bool,
    pub provider_name: Option<String>,
    pub max_context_tokens: usize,
    pub summary_max_tokens: usize,

    // NEW
    pub trigger_mode: SummaryTriggerMode,
    pub message_window_threshold: Option<usize>,
    pub token_threshold_percent: Option<f64>,
    pub include_user_task: bool,
    pub include_completion_summary: bool,
    pub quality_tracking_enabled: bool,
    pub user_rating_prompt: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryTriggerMode {
    FixedMessageCount,
    TokenThreshold,
    TurnBased,
    Hybrid, // default
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider_name: None,
            max_context_tokens: 16_000,
            summary_max_tokens: 500,
            trigger_mode: SummaryTriggerMode::Hybrid,
            message_window_threshold: Some(12),
            token_threshold_percent: Some(0.6), // 60%
            include_user_task: true,
            include_completion_summary: true,
            quality_tracking_enabled: true,
            user_rating_prompt: false, // no prompting by default
        }
    }
}
```

### A2. Hybrid Summary Trigger

**Modify:** `rustic-ai-core/src/agents/memory.rs`

Add to `AgentMemory`:
```rust
pub struct AgentMemory {
    context_window_size: usize,
    summary_enabled: bool,
    summary_max_tokens: usize,
    summary_cache_max_entries: usize,
    summary_cache: Arc<RwLock<SummaryCacheState>>,

    // NEW
    pub turn_counter: Arc<RwLock<usize>>,
    pub current_turn_messages: Arc<RwLock<usize>>,
    pub trigger_mode: SummaryTriggerMode,
    pub message_window_threshold: Option<usize>,
    pub token_threshold_percent: Option<f64>,
    pub include_user_task: bool,
    pub include_completion_summary: bool,
    pub quality_tracking_enabled: bool,
}
```

**Modify:** `build_context_window()` to check trigger:

```rust
pub async fn build_context_window(
    &self,
    messages: Vec<ChatMessage>,
    system_prompt: &str,
    summarizer: Option<&dyn ModelProvider>,
) -> Result<Vec<ChatMessage>>
{
    let context_pressure = self.calculate_context_pressure(&messages, system_prompt)?;
    let should_summarize = self.should_summarize(context_pressure, &messages).await?;

    if should_summarize {
        let summary = self.generate_hybrid_summary(&messages, summarizer).await?;
        if !summary.content.trim().is_empty() {
            context.push(summary.to_message());
        }
    }

    // ... rest of existing logic
}

async fn should_summarize(&self, pressure: f64, messages: &[ChatMessage]) -> Result<bool>
{
    let turn_count = *self.turn_counter.read().await;
    let msg_count = *self.current_turn_messages.read().await;

    Ok(match self.trigger_mode {
        SummaryTriggerMode::FixedMessageCount => {
            msg_count >= self.message_window_threshold.unwrap_or(12)
        },
        SummaryTriggerMode::TokenThreshold => {
            pressure >= self.token_threshold_percent.unwrap_or(0.6)
        },
        SummaryTriggerMode::TurnBased => turn_count > 0,
        SummaryTriggerMode::Hybrid => {
            pressure >= self.token_threshold_percent.unwrap_or(0.6)
                || msg_count >= self.message_window_threshold.unwrap_or(12)
        },
    })
}
```

**Add method:** `generate_hybrid_summary()`
```rust
async fn generate_hybrid_summary(
    &self,
    messages: &[ChatMessage],
    summarizer: Option<&dyn ModelProvider>,
) -> Result<HybridSummary>
{
    let (user_task, completions) = self.extract_task_and_completions(messages)?;

    let prompt = if self.include_user_task && self.include_completion_summary {
        format!(
            "Summarize this conversation:\n\n\
            USER REQUESTED:\n{}\n\n\
            WE COMPLETED:\n{}\n\n\
            Keep it concise (max {} tokens). Focus on decisions, code changes, and remaining work.",
            user_task,
            completions.join("\n"),
            self.summary_max_tokens
        )
    } else if self.include_user_task {
        format!("Summarize: USER REQUESTED: {}", user_task)
    } else {
        format!("Summarize: WE COMPLETED: {}", completions.join("\n"))
    };

    let content = if let Some(provider) = summarizer {
        timeout(Duration::from_secs(15), provider.generate(...))
            .await
            .map_err(|_| Error::Provider("summary timed out".to_owned()))?
            .content
    } else {
        Self::heuristic_hybrid_summary(user_task, completions)?
    };

    Ok(HybridSummary::new(
        user_task,
        completions,
        content,
        messages.len(),
        self.trigger_mode,
    ))
}

fn extract_task_and_completions(&self, messages: &[ChatMessage]) -> Result<(String, Vec<String>)>
{
    let user_task = messages
        .iter()
        .rev()
        .find(|msg| msg.role == "user")
        .map(|msg| msg.content.clone())
        .unwrap_or_default();

    let completions: Vec<String> = messages
        .iter()
        .filter(|msg| msg.role == "assistant" || msg.role == "tool")
        .map(|msg| {
            let compact = msg.content.split_whitespace().collect::<Vec<_>>().join(" ");
            compact.chars().take(200).collect()
        })
        .take(8)
        .collect();

    Ok((user_task, completions))
}
```

### A3. Quality Tracking

**Add to storage model:** `rustic-ai-core/src/storage/model.rs`
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryQuality {
    pub id: Uuid,
    pub session_id: Uuid,
    pub summary_key: String,
    pub rating: i8, // -1 poor, 0 ok, 1 good
    pub implicit: bool, // true if not user-prompted
    pub acceptance_count: usize, // number of times summary was reused
    pub rejection_count: usize, // number of times summary was regenerated
    pub created_at: DateTime<Utc>,
}
```

**Add SQLite migration:**
```sql
CREATE TABLE summary_quality (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    summary_key TEXT NOT NULL,
    rating INTEGER NOT NULL,
    implicit BOOLEAN NOT NULL,
    acceptance_count INTEGER NOT NULL DEFAULT 0,
    rejection_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_summary_quality_key ON summary_quality(summary_key);
CREATE INDEX idx_summary_quality_session ON summary_quality(session_id);
```

### A4. Events

**Add to:** `rustic-ai-core/src/events/types.rs`
```rust
pub enum Event {
    // ... existing ...
    SummaryGenerated {
        session_id: String,
        agent: String,
        trigger: SummaryTriggerMode,
        message_count: usize,
        token_pressure: f64,
        summary_length: usize,
        has_user_task: bool,
        has_completion_summary: bool,
    },
    SummaryQualityUpdated {
        session_id: String,
        summary_key: String,
        rating: i8,
        implicit: bool,
        acceptance_count: usize,
    },
}
```

---

## FEATURE B: TODO Tracking (3-4 days)

### B1. Config Schema

**Modify:** `rustic-ai-core/src/config/schema.rs`

```rust
pub struct FeatureConfig {
    // ... existing ...
    pub todo_tracking_enabled: bool,
}

pub struct AgentConfig {
    // ... existing ...
    pub auto_create_todos: bool,    // default: true
    pub todo_project_scope: bool,    // default: true (create both)
}
```

### B2. Data Model

**Add to:** `rustic-ai-core/src/storage/model.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: Uuid,
    pub project_id: Option<String>,  // project-wide TODO
    pub session_id: Option<Uuid>,   // session-scoped TODO
    pub parent_id: Option<Uuid>,   // hierarchical link
    pub title: String,
    pub description: Option<String>,
    pub status: TodoStatus,
    pub priority: TodoPriority,
    pub tags: Vec<String>,
    pub metadata: Option<TodoMetadata>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Todo,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoMetadata {
    pub files: Vec<String>,
    pub tools: Vec<String>,
    pub routing_trace_id: Option<Uuid>,
    pub sub_agent_output_id: Option<Uuid>,
    pub summary_id: Option<String>,
    pub reason: Option<String>, // for blocked status
}
```

### B3. Storage Implementation

**SQLite migration:**
```sql
CREATE TABLE todos (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    session_id TEXT,
    parent_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    priority TEXT NOT NULL,
    tags TEXT NOT NULL,
    metadata TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX idx_todos_project ON todos(project_id, status);
CREATE INDEX idx_todos_session ON todos(session_id, status);
CREATE INDEX idx_todos_parent ON todos(parent_id);
CREATE INDEX idx_todos_created ON todos(created_at);

CREATE TRIGGER update_todo_updated
    AFTER UPDATE ON todos
    BEGIN
        UPDATE todos SET updated_at = datetime('now') WHERE id = NEW.id;
    END;
```

**Add to StorageBackend trait:** `rustic-ai-core/src/storage/mod.rs`
```rust
#[async_trait]
pub trait StorageBackend {
    // ... existing ...

    async fn create_todo(&self, todo: Todo) -> Result<Todo>;
    async fn list_todos(&self, filter: TodoFilter) -> Result<Vec<Todo>>;
    async fn update_todo(&self, id: Uuid, updates: TodoUpdate) -> Result<Todo>;
    async fn delete_todo(&self, id: Uuid) -> Result<()>;
    async fn get_todo(&self, id: Uuid) -> Result<Option<Todo>>;
    async fn complete_todo_chain(&self, id: Uuid) -> Result<Vec<Todo>>; // complete parent if all children done
}

pub struct TodoFilter {
    pub project_id: Option<String>,
    pub session_id: Option<Uuid>,
    pub status: Option<TodoStatus>,
    pub priority: Option<TodoPriority>,
    pub tags: Option<Vec<String>>,
    pub parent_id: Option<Uuid>,
}

pub struct TodoUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TodoStatus>,
    pub priority: Option<TodoPriority>,
    pub tags: Option<Vec<String>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub metadata: Option<TodoMetadata>,
}
```

### B4. Hierarchy Logic

**Add method:** `complete_todo_chain()`
```rust
async fn complete_todo_chain(&self, id: Uuid) -> Result<Vec<Todo>>
{
    // Mark this TODO as completed
    let todo = self.get_todo(id).await?.ok_or(Error::NotFound)?;
    let mut updated = self.update_todo(
        id,
        TodoUpdate {
            status: Some(TodoStatus::Completed),
            completed_at: Some(Utc::now()),
            ..Default::default()
        }
    ).await?;

    let mut completed_chain = vec![updated.clone()];

    // Propagate to parent
    if let Some(parent_id) = updated.parent_id {
        let siblings = self.list_todos(TodoFilter {
            parent_id: Some(parent_id),
            ..Default::default()
        }).await?;

        let all_completed = siblings.iter().all(|s| s.status == TodoStatus::Completed);

        if all_completed {
            let parent_updated = self.complete_todo_chain(parent_id).await?;
            completed_chain.extend(parent_updated);
        }
    }

    Ok(completed_chain)
}
```

### B5. CLI Commands

**Add to:** `frontend/rustic-ai-cli/src/cli.rs`
```rust
pub enum Command {
    // ... existing ...
    Todo {
        #[command(subcommand)]
        command: TodoCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TodoCommand {
    List {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        status: Option<TodoStatus>,
        #[arg(long)]
        priority: Option<TodoPriority>,
        #[arg(long)]
        show_metadata: bool,
    },
    Add {
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        priority: Option<TodoPriority>,
        #[arg(long)]
        tags: Option<Vec<String>>,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long)]
        project: Option<bool>, // true = project TODO, false = session TODO
    },
    Update {
        id: String,
        #[arg(long)]
        status: Option<TodoStatus>,
        #[arg(long)]
        priority: Option<TodoPriority>,
        #[arg(long)]
        tags: Option<Vec<String>>,
    },
    Complete {
        id: String,
    },
    Delete {
        id: String,
    },
}
```

### B6. Session Status Integration

**Modify:** Session status output in CLI

```
TODO List (session: abc123, project: myproject)

Project TODOs (3):
  [🔴 Critical] Implement parallel sub-agent execution
    ├─ [ ] Add parallel execution to coordinator
    ├─ [ ] Add semaphore-based concurrency control
    └─ [x] Add config schema (completed 2025-02-12)

  [🟡 High] Implement dynamic routing
    ├─ [ ] Add router module
    └─ [ ] Add routing traces

Session TODOs (active: 2):
  [ ] Test parallel sub-agents with semaphore
  [ ] Verify caching works correctly

Session TODOs (completed: 1):
  [x] Update config schema (linked to project TODO)
```

---

## FEATURE C: Sub-Agent Orchestration v2 (6-8 days)

### C1. Config Schema

**Modify:** `rustic-ai-core/src/config/schema.rs`

```rust
pub struct FeatureConfig {
    // ... existing ...
    pub sub_agent_parallel_enabled: bool,
    pub sub_agent_output_caching_enabled: bool,
}

pub struct AgentConfig {
    // ... existing ...
    pub parallel_sub_agent_enabled: bool,
    pub aggressive_delegation_policy: Option<DelegationPolicy>,
    pub sub_agent_max_parallel_tasks: Option<usize>, // None = unlimited
    pub sub_agent_output_cache_mode: SubAgentCacheMode,
    pub sub_agent_output_cache_ttl_secs: Option<u64>,
    pub sub_agent_parallel_progress_enabled: bool,  // default: true
    pub sub_agent_parallel_detailed_logs: bool,    // default: false
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationPolicy {
    Conservative,
    Moderate,
    Aggressive, // default
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentCacheMode {
    ExactMatch,
    Semantic,
    Hybrid, // default
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            // ... existing ...
            parallel_sub_agent_enabled: false,
            aggressive_delegation_policy: Some(DelegationPolicy::Aggressive),
            sub_agent_max_parallel_tasks: Some(8),
            sub_agent_output_cache_mode: SubAgentCacheMode::Hybrid,
            sub_agent_output_cache_ttl_secs: Some(3600), //1 hour
            sub_agent_parallel_progress_enabled: true,
            sub_agent_parallel_detailed_logs: false,
        }
    }
}
```

### C2. Parallel Execution

**Modify:** `rustic-ai-core/src/agents/coordinator.rs`

```rust
async fn run_parallel_sub_agents(
    &self,
    requests: Vec<SubAgentRequest>,
    tx: mpsc::Sender<Event>,
    max_parallelism: Option<usize>,
) -> Vec<Result<SubAgentResult>>
{
    let semaphore = Arc::new(Semaphore::new(
        max_parallelism.unwrap_or(requests.len())
    ));

    let mut tasks = Vec::with_capacity(requests.len());

    for request in requests {
        let semaphore = semaphore.clone();
        let coordinator = self.clone();
        let tx = tx.clone();

        tasks.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            coordinator.run_sub_agent(request, tx).await
        }));
    }

    let results = join_all(tasks).await;

    // Preserve order
    results.into_iter().map(|r| r.unwrap()).collect()
}
```

### C3. Aggressive Delegation

**Add method to:** `rustic-ai-core/src/agents/behavior.rs`

```rust
async fn delegate_sub_agents_aggressively(
    &self,
    task_description: String,
    context_pressure: f64,
) -> Result<Vec<SubAgentResult>>
{
    let policy = self.config.aggressive_delegation_policy
        .unwrap_or(DelegationPolicy::Aggressive);

    // Skip if pressure is low and policy is conservative
    if policy == DelegationPolicy::Conservative && context_pressure < 0.5 {
        return Ok(vec![]);
    }

    // Split task based on keywords
    let sub_tasks = self.split_task_by_keywords(&task_description)?;

    // Select target agents for each sub-task
    let requests = sub_tasks.into_iter()
        .map(|sub_task| self.create_sub_agent_request(sub_task))
        .collect::<Result<Vec<_>>>()?;

    // Run in parallel if enabled
    if self.config.parallel_sub_agent_enabled {
        let max_parallel = self.config.sub_agent_max_parallel_tasks;
        self.coordinator.run_parallel_sub_agents(requests, tx, max_parallel).await
    } else {
        // Sequential fallback
        let mut results = Vec::new();
        for req in requests {
            results.push(self.coordinator.run_sub_agent(req, tx.clone()).await?);
        }
        Ok(results)
    }
}
```

### C4. Output Caching (Hybrid Mode)

**Add to storage model:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentOutput {
    pub id: Uuid,
    pub caller_agent: String,
    pub target_agent: String,
    pub task_key: String,         // Exact match key
    pub task_type: Option<String>, // Semantic cache key
    pub output: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}
```

**SQLite migration:**
```sql
CREATE TABLE sub_agent_outputs (
    id TEXT PRIMARY KEY,
    caller_agent TEXT NOT NULL,
    target_agent TEXT NOT NULL,
    task_key TEXT NOT NULL,
    task_type TEXT,
    output TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    metadata TEXT
);

CREATE INDEX idx_sub_agent_outputs_exact ON sub_agent_outputs(caller_agent, target_agent, task_key);
CREATE INDEX idx_sub_agent_outputs_semantic ON sub_agent_outputs(caller_agent, target_agent, task_type);
CREATE INDEX idx_sub_agent_outputs_expires ON sub_agent_outputs(expires_at);
```

**Cache logic:**
```rust
async fn get_cached_output(
    &self,
    caller: &str,
    target: &str,
    task_key: &str,
    task_type: Option<&str>,
    mode: SubAgentCacheMode,
) -> Result<Option<String>>
{
    let storage = &self.session_manager.storage;

    match mode {
        SubAgentCacheMode::ExactMatch => {
            storage.get_sub_agent_output_exact(caller, target, task_key).await
        },
        SubAgentCacheMode::Semantic => {
            if let Some(ty) = task_type {
                storage.get_sub_agent_output_semantic(caller, target, ty).await
            } else {
                Ok(None)
            }
        },
        SubAgentCacheMode::Hybrid => {
            // Try exact first
            if let Some(output) = storage.get_sub_agent_output_exact(caller, target, task_key).await? {
                return Ok(Some(output));
            }
            // Fall back to semantic
            if let Some(ty) = task_type {
                storage.get_sub_agent_output_semantic(caller, target, ty).await
            } else {
                Ok(None)
            }
        },
    }
}
```

### C5. IO Contracts

**Modify:** `rustic-ai-core/src/tools/sub_agent.rs`

```rust
#[derive(Debug, Clone, Default, Deserialize)]
struct SubAgentArgs {
    target_agent: String,
    task: String,
    // ... existing ...
    expected_output_schema: Option<serde_json::Value>,
    max_output_tokens: Option<usize>,
}

impl SubAgentTool {
    fn validate_output(&self, output: &str, schema: Option<&Value>) -> Result<()>
    {
        if let Some(schema) = schema {
            let parsed: Value = serde_json::from_str(output)
                .map_err(|_| Error::Tool("output is not valid JSON".to_owned()))?;

            jsonschema::validate(schema, &parsed)
                .map_err(|e| Error::Tool(format!("output schema validation failed: {}", e)))?;
        }
        Ok(())
    }

    fn truncate_output(&self, output: String, max_tokens: Option<usize>) -> String
    {
        if let Some(max) = max_tokens {
            let char_limit = max * 4; // rough estimate
            output.chars().take(char_limit).collect()
        } else {
            output
        }
    }
}
```

### C6. Parallel Visibility

**Add events:** `rustic-ai-core/src/events/types.rs`
```rust
pub enum Event {
    // ... existing ...
    SubAgentParallelStarted {
        session_id: String,
        caller_agent: String,
        task_count: usize,
        max_parallelism: usize,
    },
    SubAgentParallelProgress {
        session_id: String,
        caller_agent: String,
        completed: usize,
        total: usize,
    },
    SubAgentDetailedLog {
        session_id: String,
        caller_agent: String,
        target_agent: String,
        log_level: String,
        message: String,
    },
    SubAgentOutputCacheHit {
        session_id: String,
        caller_agent: String,
        target_agent: String,
        task_key: String,
        cache_mode: SubAgentCacheMode,
    },
}
```

---

## FEATURE D: Dynamic Routing (5-7 days)

### D1. Config Schema

**Add to:** `rustic-ai-core/src/config/schema.rs`

```rust
pub struct Config {
    // ... existing ...
    pub dynamic_routing: DynamicRoutingConfig,
}

pub struct DynamicRoutingConfig {
    pub enabled: bool,
    pub routing_policy: RoutingPolicy,
    pub task_keywords: HashMap<String, Vec<String>>,
    pub fallback_agent: String,
    pub context_pressure_threshold: f64,
    pub routing_trace_enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPolicy {
    TaskType,
    AgentCapabilities,
    ContextPressure,
    Hybrid, // default
}

impl Default for DynamicRoutingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            routing_policy: RoutingPolicy::Hybrid,
            task_keywords: HashMap::from([
                ("testing".to_string(), vec!["test".to_string(), "spec".to_string(), "assert".to_string()]),
                ("debugging".to_string(), vec!["debug".to_string(), "error".to_string(), "fix".to_string()]),
                ("build".to_string(), vec!["build".to_string(), "compile".to_string(), "release".to_string()]),
                ("implement".to_string(), vec!["implement".to_string(), "add".to_string(), "create".to_string()]),
            ]),
            fallback_agent: "general".to_string(),
            context_pressure_threshold: 0.7,
            routing_trace_enabled: true,
        }
    }
}
```

### D2. Router Module

**Add:** `rustic-ai-core/src/routing/mod.rs`

```rust
use crate::agents::registry::AgentRegistry;
use crate::config::schema::{DynamicRoutingConfig, RoutingPolicy, RoutingReason};
use crate::error::Result;

pub struct Router {
    registry: Arc<AgentRegistry>,
    config: DynamicRoutingConfig,
}

#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub target_agent: String,
    pub reason: RoutingReason,
    pub confidence: f64,
    pub alternatives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingReason {
    TaskTypeMatch { task_type: String },
    CapabilityMatch { capability: String },
    ContextPressureSplit { pressure: f64 },
    Fallback,
}

impl Router {
    pub fn new(registry: Arc<AgentRegistry>, config: DynamicRoutingConfig) -> Self {
        Self { registry, config }
    }

    pub async fn route(
        &self,
        task_description: String,
        current_context_usage: f64,
        caller_agent: String,
    ) -> Result<RoutingDecision>
    {
        match self.config.routing_policy {
            RoutingPolicy::TaskType => self.route_by_task_type(&task_description).await,
            RoutingPolicy::AgentCapabilities => self.route_by_capabilities(&task_description).await,
            RoutingPolicy::ContextPressure => self.route_by_context_pressure(current_context_usage).await,
            RoutingPolicy::Hybrid => self.route_hybrid(task_description, current_context_usage).await,
        }
    }

    async fn route_hybrid(
        &self,
        task: String,
        pressure: f64,
    ) -> Result<RoutingDecision>
    {
        // 1. Extract task type
        let task_type = self.analyze_task_type(&task);

        // 2. Filter agents by capabilities
        let candidates = self.filter_agents_by_capabilities(&task, &task_type);

        // 3. Score each candidate
        let scored = candidates.into_iter()
            .map(|agent| {
                let score = self.score_agent(&agent, &task, &task_type, pressure);
                (agent.name.clone(), score)
            })
            .collect::<Vec<_>>();

        // 4. Select best agent
        if let Some((best_agent, confidence)) = scored.into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()) {
            if confidence >= 0.3 {
                return Ok(RoutingDecision {
                    target_agent: best_agent,
                    reason: RoutingReason::CapabilityMatch { capability: task_type.unwrap_or_default() },
                    confidence,
                    alternatives: self.get_alternatives(&scored, &best_agent),
                });
            }
        }

        // 5. Fallback
        Ok(RoutingDecision {
            target_agent: self.config.fallback_agent.clone(),
            reason: RoutingReason::Fallback,
            confidence: 0.0,
            alternatives: vec![],
        })
    }
}
```

### D3. Routing Trace Storage

**Add to model:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingTrace {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_description: String,
    pub caller_agent: String,
    pub target_agent: String,
    pub reason: RoutingReason,
    pub confidence: f64,
    pub alternatives: Vec<String>,
    pub context_pressure: f64,
    pub created_at: DateTime<Utc>,
}
```

**SQLite migration:**
```sql
CREATE TABLE routing_traces (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    task_description TEXT NOT NULL,
    caller_agent TEXT NOT NULL,
    target_agent TEXT NOT NULL,
    reason TEXT NOT NULL,
    confidence REAL NOT NULL,
    alternatives TEXT NOT NULL,
    context_pressure REAL NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_routing_traces_session ON routing_traces(session_id, created_at);
```

### D4. CLI Diagnostics

**Add to:** `frontend/rustic-ai-cli/src/cli.rs`
```rust
pub enum Command {
    // ... existing ...
    Routing {
        #[command(subcommand)]
        command: RoutingCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum RoutingCommand {
    Trace {
        session_id: String,
        limit: Option<usize>,
    },
    Analyze {
        task: String,
        #[arg(long)]
        context_pressure: Option<f64>,
    },
}
```

---

## IMPLEMENTATION PHASES

### Phase A: Feature A (Context Summary) - Week 1
**Day 1-2:**
- Config schema updates
- Hybrid trigger logic in `memory.rs`
- Extract task and completions

**Day 3:**
- Quality tracking model + storage
- SQLite/Postgres migrations

**Day 4:**
- Events for summary generation
- CLI rendering of summary events

### Phase B: Feature B (TODO Tracking) - Week 2
**Day 1-2:**
- Config schema
- Data model (Todo, TodoMetadata, filters)
- Storage trait methods

**Day 3:**
- SQLite/Postgres migrations
- Hierarchy logic (complete_todo_chain)

**Day 4:**
- CLI commands (list/add/update/complete/delete)
- Session status integration

### Phase C: Feature C (Sub-Agent v2) - Weeks 3-4
**Week 3 - Day 1-3:**
- Config schema (parallel, caching, delegation policies)
- Parallel execution in coordinator
- Aggressive delegation heuristics

**Week 3 - Day 4-5:**
- Hybrid output caching (exact + semantic)
- IO contracts (schema validation, token truncation)
- Cache pruning logic

**Week 4 - Day 1-2:**
- Events (parallel start/progress, cache hits)
- CLI visibility (progress bars, detailed logs toggle)

**Week 4 - Day 3-4:**
- Auto-TODO integration (sub-agents spawn child TODOs)
- E2E testing for parallel scenarios

### Phase D: Feature D (Dynamic Routing) - Weeks 5-6
**Week 5 - Day 1-3:**
- Config schema (routing policy, fallback, task keywords)
- Router module (hybrid algorithm)
- Task type analysis + capability scoring

**Week 5 - Day 4-5:**
- Routing trace storage + migrations
- CLI diagnostics (trace/analyze commands)

**Week 6 - Day 1-3:**
- Router integration with Agent behavior
- Context pressure input from memory

**Week 6 - Day 4-5:**
- Link routing traces to TODOs (metadata)
- Per-task-type semantic caching integration
- E2E testing for routing scenarios

### Phase E: Cross-Feature Integration - Week 7
**Day 1-2:**
- TODO + Sub-Agent integration (child TODOs, status propagation)
- TODO + Routing integration (trace metadata)
- TODO + Summary integration (summary metadata)

**Day 3:**
- Summary quality tracking integration (implicit acceptance/rejection)

**Day 4:**
- Parallel visibility optimization (progress bars, performance)

**Day 5:**
- Full E2E tests for combined scenarios

---

## TESTING STRATEGY

### Unit Tests (per feature)
- Config parsing/validation
- Storage CRUD operations
- Router scoring algorithms
- Summary trigger logic
- Cache key generation

### Integration Tests
- Parallel sub-agent execution with semaphores
- Routing decisions across different task types
- TODO hierarchy (parent/child status propagation)
- Summary generation with provider fallbacks

### E2E Scenarios
1. **Parallel sub-agents with TODOs:**
   - Agent delegates to 3 parallel sub-agents
   - TODOs created for each
   - Sub-agents complete → TODOs marked done
   - Parent TODO marked complete

2. **Context pressure triggers summary:**
   - Long conversation fills context window
   - Hybrid summary triggered (token threshold)
   - Summary quality tracked
   - Next turn uses cached summary

3. **Routing with fallback:**
   - Unknown task type sent to router
   - Router tries to match by capabilities
   - No agent with confidence > 0.3
   - Fallback agent used
   - Routing trace stored + linked to TODO

4. **All features combined:**
   - User: "Test the API and build the project"
   - Router analyzes → splits into "testing" + "build"
   - Parallel sub-agents spawned (test agent + build agent)
   - TODOs created (project parent, session children)
   - Sub-agents complete → outputs cached (hybrid mode)
   - Context pressure rises → summary generated
   - TODOs completed → status propagated to parent

---

## VALIDATION COMMANDS

After each phase:

```bash
# Format
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all

# Build
cargo build --workspace

# Lint
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Test
cargo test -p rustic-ai-core agents::memory -- --nocapture
cargo test -p rustic-ai-core routing:: -- --nocapture
cargo test -p rustic-ai-core storage:: -- --nocapture
cargo test -p rustic-ai-cli cli:: -- --nocapture
```

CLI validation:
```bash
# Summary
cargo run -p rustic-ai-cli -- --config config.json validate-config --strict

# TODOs
cargo run -p rustic-ai-cli -- --config config.json todo list --session-id <id>
cargo run -p rustic-ai-cli -- --config config.json todo add "Test parallel sub-agents" --priority high

# Routing
cargo run -p rustic-ai-cli -- --config config.json routing analyze "Debug database connection" --context-pressure 0.8
cargo run -p rustic-ai-cli -- --config config.json routing trace <session-id>
```

---

## FILES TO MODIFY

### Core
- `rustic-ai-core/src/config/schema.rs` (all features)
- `rustic-ai-core/src/agents/memory.rs` (summary)
- `rustic-ai-core/src/agents/coordinator.rs` (sub-agent parallel)
- `rustic-ai-core/src/agents/behavior.rs` (delegation, TODO auto-creation)
- `rustic-ai-core/src/storage/model.rs` (TODO, summary quality, routing trace, sub-agent output)
- `rustic-ai-core/src/storage/sqlite/backend.rs` (migrations, implementations)
- `rustic-ai-core/src/storage/postgres/backend.rs` (migrations, implementations)
- `rustic-ai-core/src/storage/mod.rs` (trait methods)
- `rustic-ai-core/src/events/types.rs` (new events)
- `rustic-ai-core/src/tools/sub_agent.rs` (IO contracts, caching)
- `rustic-ai-core/src/routing/mod.rs` (NEW module)

### CLI
- `frontend/rustic-ai-cli/src/cli.rs` (commands)
- `frontend/rustic-ai-cli/src/main.rs` (handlers)
- `frontend/rustic-ai-cli/src/renderer.rs` (events display)

### Docs
- `docs/DECISIONS.md` (ADRs for caching, hierarchy, routing) ✅ DONE
- `TODO.md` (mark items 25, 26, 28, 29 as in-progress) ✅ DONE
- `docs/implementation-plan-four-optional-features.md` (THIS FILE) ✅ DONE

---

## RISKS AND MITIGATIONS

| Risk | Impact | Mitigation |
|-------|---------|-------------|
| Parallel sub-agent deadlocks | High | Timeouts (15s per sub-agent), bounded semaphores, cancel tokens |
| Semantic cache staleness | Medium | TTL pruning (1hr), fallback to exact match, cache hit tracking |
| TODO hierarchy cycles | Low | Prevent parent = child, validate on insert |
| Routing oscillation | Medium | Cache routing decisions for 5min (hysteresis) |
| Summary quality degradation | Medium | Fallback to heuristic, track implicit quality (accept/reject) |
| Performance from detailed logs | Low | Default detailed_logs=false, make configurable |

---

## CROSS-FEATURE INTEGRATION

### TODO + Sub-Agent Integration
- Sub-agent starts → create "In Progress" TODO (if `auto_create_todos`)
- Sub-agent completes → mark TODO as `Completed`
- Sub-agent fails → mark TODO as `Blocked`, set `metadata.reason`
- Parallel sub-agents → create child TODOs under parent task

### TODO + Routing Integration
- Router selects agent → create TODO with `metadata.routing_trace_id`
- Router uses fallback agent → tag TODO with `tags = ["fallback"]`

### TODO + Context Summary Integration
- Summary generated → if TODOs marked completed, add summary to `metadata.summary`

---

## ADRS CREATED

- ADR-0028: Four Optional Features (Summary, TODO, Sub-Agent v2, Dynamic Routing)
- ADR-0029: Sub-Agent Caching Strategy (Hybrid: Exact + Semantic)
- ADR-0030: TODO Hierarchy Model (Parent-Child Session ↔ Project Links)
- ADR-0031: Dynamic Routing Fallback and Confidence Thresholding

---

## IMPLEMENTATION COMPLETE

This plan is comprehensive, addresses all your requirements, and includes:
- ✅ Fully optional features (toggleable via `FeatureConfig`)
- ✅ Sub-agent orchestration v2 (parallel, hybrid caching, visibility, auto-TODOs)
- ✅ Dynamic routing (hybrid policy, fallback agent, tracing)
- ✅ Aggressive context summary (hybrid trigger, quality tracking)
- ✅ TODO tracking (session + project, hierarchy, metadata)
- ✅ Cross-feature integration (all features work together)
- ✅ Implementation phases (A → B → C → D → E)
- ✅ Testing strategy (unit + integration + E2E)
- ✅ Validation commands
- ✅ ADRs documented

**Next step:** Expand automated test coverage for these features and split follow-up hardening work from TODO item 24.
