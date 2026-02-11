# Agent-to-Agent Calling Protocol (OpenCode-Style)

**Date:** 2026-02-11
**Status:** Planned
**Priority:** HIGH
**Reference:** Gap 6 from `implementation-gaps.md`

---

## Overview

This document defines the OpenCode-style agent-to-agent calling protocol for Rustic-AI.

### Key Principles

1. **Minimal Context Transfer**: Agent A passes only what Agent B needs
2. **No Duplicates**: Avoid sending the same context to multiple agents
3. **No Context Bloat**: Respect context window limits rigorously
4. **Filtered Workspace**: Send only relevant workspace information
5. **Result Return**: Agent B returns results to Agent A for continuation

### OpenCode Approach

In OpenCode, when Agent A calls Agent B:

```
Agent A:
├─ Analyzes task
├─ Identifies need for specialist (Agent B)
├─ Extracts minimal context:
│  ├─ Last N messages (e.g., last 5)
│  ├─ Current task description
│  ├─ Relevant workspace context (files being edited)
│  └─ Filters out irrelevant history
└─ Calls Agent B with:
   ├─ task: Clear description of what Agent B should do
   ├─ context: Filtered subset of conversation
   ├─ workspace: Minimal workspace info (files, structure)
   └─ max_tokens: Explicit budget (e.g., 4000)

Agent B:
├─ Receives filtered context (not full conversation)
├─ Has own context window
├─ Works on task with only relevant information
├─ Returns results:
│  ├─ success: true/false
│  ├─ output: Response or work product
│  ├─ tool_calls: Tools used
│  └─ tokens_used: Tokens consumed

Agent A:
├─ Receives Agent B's result
├─ Integrates result into its reasoning
├─ Continues task without duplicate context
└─ Stores sub-agent call in shared session history
```

### Why This Works

1. **Prevents Bloat**:
   - Agent B doesn't see Agent A's full history
   - Agent B doesn't see Agent A's tool calls
   - Agent B doesn't see unrelated conversation

2. **Reduces Duplication**:
   - Each agent has its own focused context
   - Shared session only stores interaction, not duplicates
   - Results are passed by value, not by copying context

3. **Improves Performance**:
   - Fewer tokens wasted on irrelevant history
   - Faster response times (smaller context)
   - Better focus for each agent

4. **Enables Specialization**:
   - "planner" agents can call "coder" agents
   - "coder" agents can call "reviewer" agents
   - Each agent has tools/prompt optimized for its role
   - Context is filtered to match the role

---

## Implementation Design

### 1. Sub-Agent Call Tool

```rust
// Location: rustic-ai-core/src/tools/sub_agent.rs

use serde::{Deserialize, Serialize};
use crate::AgentCoordinator;

/// Tool for calling sub-agents with filtered context
pub struct SubAgentTool {
    coordinator: Arc<AgentCoordinator>,
}

/// Arguments for sub-agent call
#[derive(Debug, Clone, Deserialize)]
pub struct SubAgentArgs {
    /// Target agent name (must be defined in config)
    pub agent_name: String,

    /// Clear task description for the sub-agent
    pub task: String,

    /// Context filter to limit what's passed
    pub context: Option<ContextFilter>,

    /// Optional input data for the task
    pub input: Option<serde_json::Value>,

    /// Maximum tokens for sub-agent's context (default: 4000)
    pub max_context_tokens: Option<usize>,

    /// Override agent's default tool whitelist (optional)
    pub allowed_tools: Option<Vec<String>>,

    /// Allow sub-agent to make nested calls (default: false for safety)
    pub allow_nested_calls: Option<bool>,
}

/// Context filter options
#[derive(Debug, Clone, Deserialize)]
pub struct ContextFilter {
    /// Include last N messages from current session
    pub last_messages: Option<usize>,

    /// Include only messages matching specific roles (user, assistant, tool, system)
    pub roles: Option<Vec<String>>,

    /// Include messages containing specific keywords
    pub keywords: Option<Vec<String>>,

    /// Include session summary if available
    pub include_summary: bool,

    /// Include workspace context (files, structure, project info)
    pub include_workspace: bool,

    /// Include specific files from workspace
    pub workspace_files: Option<Vec<String>>,

    /// Maximum age of context to include (e.g., only last 10 minutes)
    pub max_age_seconds: Option<u64>,
}

/// Result from sub-agent call
#[derive(Debug, Clone, Serialize)]
pub struct SubAgentResult {
    /// Agent that was called
    pub agent_name: String,

    /// Whether the call succeeded
    pub success: bool,

    /// Agent's response or work product
    pub output: String,

    /// Tools that the sub-agent used
    pub tool_calls: Vec<String>,

    /// Tokens consumed by sub-agent
    pub tokens_used: usize,

    /// How long the sub-agent took
    pub duration_seconds: u64,

    /// Any errors that occurred
    pub errors: Vec<String>,
}
```

### 2. Filtered Context Builder

```rust
// Location: rustic-ai-core/src/agents/context_builder.rs

use crate::providers::types::ChatMessage;
use crate::SessionManager;

pub struct ContextBuilder;

impl ContextBuilder {
    /// Build filtered context for sub-agent call (OpenCode-style)
    pub async fn build_filtered_context(
        &self,
        session_id: uuid::Uuid,
        filter: &ContextFilter,
        max_tokens: usize,
        session_manager: &SessionManager,
    ) -> Result<FilteredContext> {
        let messages = session_manager
            .get_session_messages(session_id)
            .await?;

        // Step 1: Filter by time (max_age_seconds)
        let filtered_by_time = if let Some(max_age) = filter.max_age_seconds {
            let cutoff = Utc::now() - chrono::Duration::seconds(max_age as i64);
            messages.into_iter()
                .filter(|m| m.created_at > cutoff)
                .collect()
        } else {
            messages
        };

        // Step 2: Filter by roles
        let filtered_by_role = if let Some(roles) = &filter.roles {
            filtered_by_time.into_iter()
                .filter(|m| roles.contains(&m.role))
                .collect()
        } else {
            filtered_by_time
        };

        // Step 3: Filter by keywords
        let filtered_by_keywords = if let Some(keywords) = &filter.keywords {
            filtered_by_role.into_iter()
                .filter(|m| {
                    keywords.iter().any(|kw| m.content.to_lowercase().contains(kw))
                })
                .collect()
        } else {
            filtered_by_role
        };

        // Step 4: Take last N messages
        let mut selected = if let Some(n) = filter.last_messages {
            let take = filtered_by_keywords.len().saturating_sub(n);
            filtered_by_keywords.into_iter().skip(take).collect()
        } else {
            filtered_by_keywords
        };

        // Step 5: Add summary if requested
        let summary_context = if filter.include_summary {
            if let Ok(summary) = session_manager.get_session_summary(session_id).await {
                format!("\n[Conversation Summary]\n{}\n", summary)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Step 6: Add workspace context if requested
        let workspace_context = if filter.include_workspace {
            self.build_workspace_context(session_manager, session_id, filter).await?
        } else {
            String::new()
        };

        // Step 7: Fit within max_tokens
        let mut token_count = 0usize;
        let mut fit_messages = Vec::new();

        // Count tokens for additional context
        let additional_tokens = (summary_context.len() + workspace_context.len()) / 4;

        // Take messages until we hit the limit
        for msg in selected.into_iter().rev() {
            let msg_tokens = msg.content.len() / 4;
            if token_count + additional_tokens + msg_tokens <= max_tokens {
                token_count += msg_tokens;
                fit_messages.push(msg);
            } else {
                break;
            }
        }

        fit_messages.reverse();

        Ok(FilteredContext {
            messages: fit_messages,
            summary: if summary_context.is_empty() { None } else { Some(summary_context) },
            workspace: if workspace_context.is_empty() { None } else { Some(workspace_context) },
            total_tokens: token_count + additional_tokens,
        })
    }

    /// Build workspace context (minimal, relevant)
    async fn build_workspace_context(
        &self,
        session_manager: &SessionManager,
        session_id: uuid::Uuid,
        filter: &ContextFilter,
    ) -> Result<String> {
        let mut context = String::new();

        // Get working directory
        let workdir = std::env::current_dir()?;
        context.push_str(&format!("Working directory: {}\n", workdir.display()));

        // Include specific files if requested
        if let Some(files) = &filter.workspace_files {
            context.push_str("\nRelevant files:\n");
            for file_path in files {
                let full_path = workdir.join(file_path);
                if full_path.exists() {
                    context.push_str(&format!("  - {}\n", file_path));
                }
            }
        } else {
            // List top-level structure only
            context.push_str("\nWorkspace structure:\n");
            if let Ok(entries) = std::fs::read_dir(&workdir) {
                for entry in entries.flatten().take(20) {  // Limit to prevent bloat
                    let path = entry.path();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if path.is_dir() {
                        context.push_str(&format!("  [DIR] {}\n", name));
                    } else if path.is_file() {
                        context.push_str(&format!("  [FILE] {}\n", name));
                    }
                }
            }
        }

        // Include project info if available
        if let Ok(project) = session_manager.get_project_info(session_id).await {
            context.push_str(&format!("\nProject: {}\n", project.name));
            if !project.tech_stack.is_empty() {
                context.push_str(&format!("Tech stack: {}\n", project.tech_stack.join(", ")));
            }
        }

        Ok(context)
    }
}

#[derive(Debug, Clone)]
pub struct FilteredContext {
    pub messages: Vec<ChatMessage>,
    pub summary: Option<String>,
    pub workspace: Option<String>,
    pub total_tokens: usize,
}
```

### 3. Sub-Agent Execution with Context Isolation

```rust
// Location: rustic-ai-core/src/agents/coordinator.rs

use crate::agents::Agent;
use crate::tools::sub_agent::{SubAgentArgs, SubAgentResult, ContextFilter};
use crate::SessionManager;

impl AgentCoordinator {
    /// Execute a sub-agent call (OpenCode-style with filtered context)
    pub async fn call_sub_agent(
        &self,
        caller_agent_name: &str,
        args: SubAgentArgs,
        session_id: uuid::Uuid,
    ) -> Result<SubAgentResult> {
        // Validate caller has permission to call sub-agents
        let caller_config = self.registry.get_config(caller_agent_name)
            .ok_or_else(|| Error::NotFound(format!("Caller agent '{}' not found", caller_agent_name)))?;

        if !caller_config.allow_sub_agent_calls.unwrap_or(false) {
            return Err(Error::Permission(
                format!("Agent '{}' is not allowed to call other agents", caller_agent_name)
            ));
        }

        // Validate target agent exists
        let target_config = self.registry.get_config(&args.agent_name)
            .ok_or_else(|| Error::NotFound(format!("Target agent '{}' not found", args.agent_name)))?;

        // Validate no recursive loops (depth limit)
        if self.is_recursive_call(caller_agent_name, &args.agent_name, session_id)? {
            return Err(Error::Validation(
                "Recursive sub-agent calls detected (max depth exceeded)".to_string()
            ));
        }

        // Build filtered context for sub-agent
        let context_filter = args.context.unwrap_or_default();
        let max_tokens = args.max_context_tokens.unwrap_or(4000);

        let filtered = ContextBuilder::build_filtered_context(
            session_id,
            &context_filter,
            max_tokens,
            &self.session_manager,
        ).await?;

        // Override target agent's tools if specified
        let target_tools = if let Some(allowed) = &args.allowed_tools {
            allowed.clone()
        } else {
            target_config.tools.clone()
        };

        // Prepare messages for sub-agent
        let mut messages = filtered.messages;

        // Add workspace context if present
        if let Some(workspace) = &filtered.workspace {
            messages.insert(0, ChatMessage {
                role: "system".to_string(),
                content: format!("{}\n\n{}", target_config.system_prompt_template.as_deref().unwrap_or("You are a helpful AI assistant."), workspace),
                name: Some("workspace".to_string()),
                tool_calls: None,
            });
        } else {
            messages.insert(0, ChatMessage {
                role: "system".to_string(),
                content: target_config.system_prompt_template.as_deref().unwrap_or("You are a helpful AI assistant.").to_string(),
                name: None,
                tool_calls: None,
            });
        }

        // Add task as user message
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: format!("[Called by: {}]\nTask: {}", caller_agent_name, args.task),
            name: Some(caller_agent_name.to_string()),
            tool_calls: None,
        });

        // Get target agent
        let target_agent = self.registry.get(&args.agent_name)
            .ok_or_else(|| Error::NotFound(format!("Agent '{}' not found", args.agent_name)))?;

        // Track sub-agent call depth
        self.track_sub_agent_call(session_id, caller_agent_name, &args.agent_name)?;

        // Execute sub-agent
        let start_time = std::time::Instant::now();
        let mut errors = Vec::new();

        // Create isolated turn for sub-agent
        let result = target_agent.run_turn_with_tools(
            messages,
            target_tools,
            self.session_manager.clone(),
        ).await;

        let duration = start_time.elapsed();
        let tokens_used = self.estimate_tokens(&messages) + self.estimate_tokens(&result.output_messages);

        let (success, output, tool_calls) = match result {
            Ok(turn_result) => {
                let output = turn_result.final_response.clone();
                let tool_calls = turn_result.tool_calls.iter().map(|c| c.tool.clone()).collect();
                (true, output, tool_calls)
            }
            Err(e) => {
                errors.push(e.to_string());
                (false, String::new(), Vec::new())
            }
        };

        // Record interaction in shared session (minimal, no duplicates)
        self.record_sub_agent_interaction(
            session_id,
            caller_agent_name,
            &args.agent_name,
            &args.task,
            &output,
            success,
            duration.as_secs() as u64,
        ).await?;

        // Clear depth tracking
        self.clear_sub_agent_tracking(session_id);

        Ok(SubAgentResult {
            agent_name: args.agent_name,
            success,
            output,
            tool_calls,
            tokens_used,
            duration_seconds: duration.as_secs() as u64,
            errors,
        })
    }

    /// Prevent infinite recursive calls (max depth: 5)
    fn is_recursive_call(
        &self,
        caller: &str,
        target: &str,
        session_id: uuid::Uuid,
    ) -> Result<bool> {
        let max_depth = 5;

        // Get call chain for this session
        let chain = self.get_call_chain(session_id)?;

        // Check depth
        if chain.len() >= max_depth {
            return Ok(true);
        }

        // Check for cycles (A->B->C->A)
        if chain.contains(target) {
            return Ok(true);
        }

        Ok(false)
    }

    fn track_sub_agent_call(
        &self,
        session_id: uuid::Uuid,
        caller: &str,
        target: &str,
    ) -> Result<()> {
        // Store in temporary tracking structure
        self.call_tracker.push((session_id, caller, target));
        Ok(())
    }
}
```

### 4. Session Integration

```rust
// Location: rustic-ai-core/src/storage/sqlite.rs (or appropriate storage backend)

impl SqliteStorage {
    /// Record minimal interaction (no full context duplication)
    pub async fn record_sub_agent_interaction(
        &self,
        session_id: uuid::Uuid,
        caller: &str,
        target: &str,
        task: &str,
        output: &str,
        success: bool,
        duration_secs: u64,
    ) -> Result<()> {
        // Store only the interaction metadata, not full context
        sqlx::query!(
            r#"
            INSERT INTO sub_agent_calls (
                session_id, caller_agent, target_agent, task,
                output_preview, success, duration_secs, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            session_id,
            caller,
            target,
            task,
            &output.chars().take(200).collect::<String>(),  // Preview only
            success,
            duration_secs,
            Utc::now()
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
```

### 5. Example Usage

#### Agent A (Planner) calling Agent B (Coder)

```json
{
  "tool": "sub_agent",
  "args": {
    "agent_name": "coder",
    "task": "Implement the user_auth function in src/auth/user.rs based on these requirements:\n1. Email validation\n2. Password hashing (bcrypt)\n3. Return success/error tuple",
    "context": {
      "last_messages": 3,
      "roles": ["user", "assistant"],
      "include_workspace": true,
      "workspace_files": ["src/auth/user.rs", "src/auth/types.rs"],
      "max_age_seconds": 600
    },
    "max_context_tokens": 6000,
    "allowed_tools": ["read", "write", "edit"],
    "allow_nested_calls": false
  }
}
```

**What Agent B receives:**
- System prompt (from its config)
- Last 3 user/assistant messages (not all history)
- Task description from Agent A
- Workspace context showing:
  - Working directory
  - Relevant files only (not full workspace)
  - Project info

**What Agent B doesn't receive:**
- Agent A's previous tool calls
- Agent A's tool results
- Agent A's internal reasoning
- Irrelevant conversation history
- Duplicate workspace information

**Agent B's result returned to Agent A:**
```json
{
  "agent_name": "coder",
  "success": true,
  "output": "I've implemented the user_auth function in src/auth/user.rs with:\n- Email validation using regex\n- Password hashing with bcrypt (cost 12)\n- Returns Result<(User, UserError), UserError> tuple\n\nThe function handles edge cases for invalid inputs and database errors.",
  "tool_calls": ["read", "write", "edit"],
  "tokens_used": 3245,
  "duration_seconds": 12,
  "errors": []
}
```

---

## Safety Mechanisms

### 1. Depth Limit

- Max sub-agent call depth: 5 levels
- Prevents infinite recursion (A->B->C->A)
- Tracked per session

### 2. Time Limits

- Each sub-agent call has timeout (default: 300s)
- Configurable via max_context_tokens + agent duration caps
- Prevents runaway sub-agent processes

### 3. Permission Checks

- Caller must have `allow_sub_agent_calls: true`
- Prevents unauthorized agent delegation
- Enforced at coordinator level

### 4. Token Budget

- Explicit max_context_tokens for sub-agent
- Context builder respects limits strictly
- Prevents sub-agent from exceeding budget

### 5. Tool Whitelist Override

- Caller can restrict sub-agent's tools
- Example: planner restricts coder to only read/write tools (no shell)
- Adds safety layer for specialized roles

---

## Integration Points

### 1. Config Schema Updates

```rust
// rustic-ai-core/src/config/schema.rs

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    // ... existing fields ...

    /// Allow this agent to call other agents
    #[serde(default)]
    pub allow_sub_agent_calls: bool,

    /// Maximum sub-agent call depth (default: 5)
    #[serde(default = "5")]
    pub max_sub_agent_depth: usize,
}
```

### 2. CLI Updates

```rust
// frontend/rustic-ai-cli/src/repl.rs

// Render sub-agent call events
fn handle_event(&self, event: Event) {
    match event {
        Event::SubAgentCall { caller, target, task } => {
            println!("{} calling {} with task: {}", caller, target, task);
        }
        Event::SubAgentResult { result } => {
            println!("{} returned: {} ({} tokens, {}s)",
                result.agent_name,
                if result.success { "SUCCESS" } else { "FAILED" },
                result.tokens_used,
                result.duration_seconds
            );
            if !result.tool_calls.is_empty() {
                println!("  Tools used: {}", result.tool_calls.join(", "));
            }
        }
        // ... other events
    }
}

// Add command to view sub-agent call history
impl Repl {
    async fn cmd_sub_agent_history(&self) {
        let history = self.session_manager
            .get_sub_agent_history(self.session_id)
            .await?;

        println!("\nSub-Agent Call History:");
        for call in &history {
            println!("  [{} -> {}] {} ({})",
                call.caller_agent,
                call.target_agent,
                if call.success { "OK" } else { "FAILED" },
                call.duration_secs
            );
        }
    }
}
```

### 3. Testing

```rust
// rustic-ai-core/tests/agent_sub_agent_tests.rs

#[tokio::test]
async fn test_filtered_context_builder() {
    // Test that context builder filters correctly
    // Test that max_tokens is respected
    // Test that workspace context is included
    // Test that summaries are included when requested
}

#[tokio::test]
async fn test_recursive_call_prevention() {
    // Test that recursive calls are prevented
    // Test that depth limit is enforced
    // Test that cycles are detected
}

#[tokio::test]
async fn test_sub_agent_with_tool_restrictions() {
    // Test that allowed_tools restricts sub-agent
    // Test that sub-agent can't use restricted tools
}

#[tokio::test]
async fn test_no_context_duplication() {
    // Create session with conversation history
    // Agent A calls Agent B
    // Verify Agent B doesn't see full history
    // Verify only filtered context was sent
}
```

---

## Success Criteria

The agent-to-agent calling protocol is complete when:

- ✅ `SubAgentTool` is implemented and registered
- ✅ `ContextBuilder` builds filtered context correctly
- ✅ `AgentCoordinator` executes sub-agent calls with isolation
- ✅ Sub-agents receive only filtered context (no full history)
- ✅ Sub-agent results are returned to caller
- ✅ Depth limit prevents infinite recursion
- ✅ Token budgets are respected
- ✅ Permission checks enforce allow_sub_agent_calls
- ✅ Storage records interactions without duplication
- ✅ CLI renders sub-agent events clearly
- ✅ Integration tests pass
- ✅ Context bloat is measured and < 20% overhead vs single agent

---

## OpenCode Comparison

| Feature | OpenCode | Rustic-AI (Planned) |
|----------|-----------|------------------------|
| **Context Transfer** | Filtered, minimal | Filtered, minimal ✓ |
| **Duplicate Prevention** | Yes (no copying) | Yes (no copying) ✓ |
| **Workspace Context** | Included selectively | Included selectively ✓ |
| **Depth Limit** | Yes (max 5) | Yes (max 5) ✓ |
| **Tool Restrictions** | Caller can restrict | Caller can restrict ✓ |
| **Result Return** | Yes, structured | Yes, structured ✓ |
| **Async Support** | Yes | Yes ✓ |
| **Streaming** | Yes | Via event stream ✓ |

---

**End of Document**
