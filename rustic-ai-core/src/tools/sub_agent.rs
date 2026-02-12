use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agents::coordinator::{AgentCoordinator, SubAgentContextFilter, SubAgentRequest};
use crate::config::schema::{SubAgentCacheMode, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::storage::StorageBackend;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct SubAgentContextFilterArgs {
    last_messages: Option<usize>,
    include_roles: Option<Vec<String>>,
    include_keywords: Option<Vec<String>>,
    keyword_match_mode: Option<String>,
    include_tool_messages: Option<bool>,
    include_workspace: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ParallelSubAgentRequest {
    target_agent: String,
    task: String,
    filter: Option<SubAgentContextFilterArgs>,
    max_context_tokens: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct SubAgentArgs {
    target_agent: String,
    task: String,
    context_filter: Option<SubAgentContextFilterArgs>,
    max_context_tokens: Option<usize>,
    expected_output_schema: Option<Value>,
    max_output_tokens: Option<usize>,
    parallel_requests: Option<Vec<ParallelSubAgentRequest>>,
}

#[derive(Clone)]
pub struct SubAgentTool {
    config: ToolConfig,
    schema: Value,
    agents: Arc<StdRwLock<Option<Arc<AgentCoordinator>>>>,
    storage: Arc<dyn StorageBackend>,
    caching_enabled: bool,
    parallel_enabled: bool,
    detailed_logs_enabled: bool,
    cache_mode: SubAgentCacheMode,
    cache_ttl_secs: Option<u64>,
}

impl SubAgentTool {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: ToolConfig,
        agents: Arc<StdRwLock<Option<Arc<AgentCoordinator>>>>,
        storage: Arc<dyn StorageBackend>,
        caching_enabled: bool,
        parallel_enabled: bool,
        detailed_logs_enabled: bool,
        cache_mode: SubAgentCacheMode,
        cache_ttl_secs: Option<u64>,
    ) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "target_agent": {
                    "type": "string",
                    "description": "Configured target agent name"
                },
                "task": {
                    "type": "string",
                    "description": "Task sent to target agent"
                },
                "context_filter": {
                    "type": "object",
                    "properties": {
                        "last_messages": { "type": "integer", "minimum": 1 },
                        "include_roles": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "include_keywords": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "keyword_match_mode": {
                            "type": "string",
                            "enum": ["any", "all"]
                        },
                        "include_tool_messages": { "type": "boolean" },
                        "include_workspace": { "type": "boolean" }
                    },
                    "additionalProperties": false
                },
                "max_context_tokens": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Max estimated tokens passed to target agent context"
                },
                "expected_output_schema": {
                    "type": "object",
                    "description": "JSON schema to validate output against"
                },
                "max_output_tokens": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Max output token limit (approximate character limit)"
                },
                "parallel_requests": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "target_agent": { "type": "string" },
                            "task": { "type": "string" },
                            "filter": { "type": "object" },
                            "max_context_tokens": { "type": "integer", "minimum": 1 }
                        },
                        "required": ["target_agent", "task"]
                    },
                    "description": "Array of parallel sub-agent requests"
                }
            },
            "required": ["target_agent", "task"]
        });

        Self {
            config,
            schema,
            agents,
            storage,
            caching_enabled,
            parallel_enabled,
            detailed_logs_enabled,
            cache_mode,
            cache_ttl_secs,
        }
    }

    fn parse_args(&self, args: Value) -> Result<SubAgentArgs> {
        let parsed = serde_json::from_value::<SubAgentArgs>(args)
            .map_err(|err| Error::Tool(format!("invalid sub_agent arguments: {err}")))?;
        if parsed.target_agent.trim().is_empty() {
            return Err(Error::Tool(
                "sub_agent requires non-empty 'target_agent'".to_owned(),
            ));
        }
        if parsed.task.trim().is_empty() {
            return Err(Error::Tool(
                "sub_agent requires non-empty 'task'".to_owned(),
            ));
        }
        Ok(parsed)
    }

    async fn find_latest_routing_todo_link(
        &self,
        session_id: Uuid,
    ) -> Option<(Uuid, Option<Uuid>)> {
        let filter = crate::storage::TodoFilter {
            session_id: Some(session_id),
            tags: Some(vec!["routing".to_string()]),
            limit: Some(64),
            ..Default::default()
        };
        let mut todos = self.storage.list_todos(&filter).await.ok()?;
        todos.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        let todo = todos.into_iter().next()?;
        Some((todo.id, todo.metadata.routing_trace_id))
    }

    async fn execute_parallel_requests(
        &self,
        requests: &[ParallelSubAgentRequest],
        session_id: uuid::Uuid,
        caller_agent: &str,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let routing_link = self.find_latest_routing_todo_link(session_id).await;
        let routing_parent_id = routing_link.map(|(todo_id, _)| todo_id);
        let routing_trace_id = routing_link.and_then(|(_, trace_id)| trace_id);

        let agents = {
            let guard = self
                .agents
                .read()
                .map_err(|_| Error::Tool("sub-agent registry lock poisoned".to_owned()))?;
            guard.clone()
        }
        .ok_or_else(|| Error::Tool("agent coordinator is not attached".to_owned()))?;

        let caller_config = agents.get_agent_config(caller_agent).cloned();
        let progress_enabled = caller_config
            .as_ref()
            .map(|cfg| cfg.sub_agent_parallel_progress_enabled)
            .unwrap_or(true);
        let detailed_logs_enabled = caller_config
            .as_ref()
            .map(|cfg| cfg.sub_agent_parallel_detailed_logs)
            .unwrap_or(self.detailed_logs_enabled);
        let max_parallelism = match caller_config
            .as_ref()
            .and_then(|cfg| cfg.sub_agent_max_parallel_tasks)
        {
            Some(0) => Some(requests.len().max(1)),
            Some(value) => Some(value.max(1)),
            None => Some(8),
        };
        let auto_todos = caller_config
            .as_ref()
            .map(|cfg| cfg.auto_create_todos)
            .unwrap_or(false);

        let mut sub_agent_requests = Vec::new();
        let mut child_todo_ids: Vec<Option<Uuid>> = Vec::with_capacity(requests.len());

        let parent_todo_id = if auto_todos {
            let parent_id = Uuid::new_v4();
            let parent_todo = crate::storage::Todo {
                id: parent_id,
                project_id: None,
                session_id,
                parent_id: routing_parent_id,
                title: format!("Parallel sub-agent batch ({} tasks)", requests.len()),
                description: Some("Auto-created from parallel sub-agent request".to_string()),
                status: crate::storage::TodoStatus::InProgress,
                priority: crate::storage::TodoPriority::Medium,
                tags: vec!["sub-agent".to_string(), "parallel".to_string()],
                metadata: crate::storage::TodoMetadata {
                    tools: vec!["sub_agent".to_string()],
                    routing_trace_id,
                    ..Default::default()
                },
                created_at: Utc::now(),
                updated_at: Utc::now(),
                completed_at: None,
            };
            let _ = self.storage.create_todo(&parent_todo).await;
            Some(parent_id)
        } else {
            None
        };

        for req in requests {
            let context_filter = req
                .filter
                .as_ref()
                .map(|filter| SubAgentContextFilter {
                    last_messages: filter.last_messages,
                    include_roles: filter.include_roles.clone(),
                    include_keywords: filter.include_keywords.clone(),
                    keyword_match_mode: match filter.keyword_match_mode.as_deref() {
                        Some("all") => crate::agents::coordinator::KeywordMatchMode::All,
                        _ => crate::agents::coordinator::KeywordMatchMode::Any,
                    },
                    include_tool_messages: filter.include_tool_messages.unwrap_or(true),
                    include_workspace: filter.include_workspace,
                })
                .unwrap_or_default();

            sub_agent_requests.push(SubAgentRequest {
                session_id,
                caller_agent_name: caller_agent.to_owned(),
                target_agent_name: req.target_agent.clone(),
                task: req.task.clone(),
                current_depth: context.sub_agent_depth + 1,
                context_filter,
                max_context_tokens: req.max_context_tokens,
                cancellation_token: context.cancellation_token.clone(),
            });

            if auto_todos {
                let todo_id = Uuid::new_v4();
                let child_todo = crate::storage::Todo {
                    id: todo_id,
                    project_id: None,
                    session_id,
                    parent_id: parent_todo_id,
                    title: format!("Sub-agent '{}' task", req.target_agent),
                    description: Some(req.task.clone()),
                    status: crate::storage::TodoStatus::InProgress,
                    priority: crate::storage::TodoPriority::Medium,
                    tags: vec!["sub-agent".to_string()],
                    metadata: crate::storage::TodoMetadata {
                        tools: vec!["sub_agent".to_string()],
                        routing_trace_id,
                        ..Default::default()
                    },
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    completed_at: None,
                };
                let _ = self.storage.create_todo(&child_todo).await;
                child_todo_ids.push(Some(todo_id));
            } else {
                child_todo_ids.push(None);
            }
        }

        let results = agents
            .run_parallel_sub_agents(
                sub_agent_requests,
                tx.clone(),
                max_parallelism,
                progress_enabled,
            )
            .await?;
        let total = results.len();

        let mut outputs = std::collections::HashMap::new();
        let mut success_count = 0;
        let mut failure_count = 0;

        for (idx, result) in results.into_iter().enumerate() {
            match result {
                Ok(output) => {
                    success_count += 1;
                    outputs.insert(idx, output);
                    if let Some(todo_id) = child_todo_ids.get(idx).and_then(|value| *value) {
                        let _ = self.storage.complete_todo_chain(todo_id).await;
                    }
                }
                Err(err) => {
                    failure_count += 1;
                    outputs.insert(idx, format!("Error: {}", err));
                    if let Some(todo_id) = child_todo_ids.get(idx).and_then(|value| *value) {
                        let _ = self
                            .storage
                            .update_todo(
                                todo_id,
                                &crate::storage::TodoUpdate {
                                    status: Some(crate::storage::TodoStatus::Blocked),
                                    metadata: Some(crate::storage::TodoMetadata {
                                        tools: vec!["sub_agent".to_string()],
                                        routing_trace_id,
                                        reason: Some(err.to_string()),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                },
                            )
                            .await;
                    }
                }
            }
        }

        if let Some(parent_id) = parent_todo_id {
            if failure_count == 0 {
                let _ = self.storage.complete_todo_chain(parent_id).await;
            } else {
                let _ = self
                    .storage
                    .update_todo(
                        parent_id,
                        &crate::storage::TodoUpdate {
                            status: Some(crate::storage::TodoStatus::Blocked),
                            metadata: Some(crate::storage::TodoMetadata {
                                tools: vec!["sub_agent".to_string()],
                                routing_trace_id,
                                reason: Some(format!(
                                    "{} of {} parallel sub-agent tasks failed",
                                    failure_count, total
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        }

        if detailed_logs_enabled {
            let _ = tx.try_send(Event::SubAgentDetailedLog {
                session_id: session_id.to_string(),
                caller_agent: caller_agent.to_owned(),
                target_agent: "*".to_string(),
                log_level: "info".to_string(),
                message: format!(
                    "parallel sub-agent batch complete: {} success, {} failed",
                    success_count, failure_count
                ),
            });
        }

        let summary = json!({
            "total": total,
            "success": success_count,
            "failed": failure_count,
            "outputs": outputs,
        });

        Ok(ToolResult {
            success: failure_count == 0,
            exit_code: Some(if failure_count > 0 { 1 } else { 0 }),
            output: summary.to_string(),
        })
    }

    fn compute_task_key(
        &self,
        target_agent: &str,
        task: &str,
        context_filter: &Option<SubAgentContextFilterArgs>,
    ) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        target_agent.hash(&mut hasher);
        task.hash(&mut hasher);
        if let Some(filter) = context_filter {
            filter.last_messages.hash(&mut hasher);
        }
        format!("{:x}", hasher.finish())
    }

    async fn try_get_cached_output(
        &self,
        task_key: &str,
        caller_agent: &str,
        target_agent: &str,
        cache_mode: SubAgentCacheMode,
        task_type: Option<&str>,
    ) -> Result<Option<crate::storage::SubAgentOutput>> {
        // Try exact match first
        if let Ok(Some(cached)) = self.storage.get_sub_agent_output_exact(task_key).await {
            return Ok(Some(cached));
        }

        // If hybrid mode, try semantic fallback
        if matches!(
            cache_mode,
            SubAgentCacheMode::Hybrid | SubAgentCacheMode::Semantic
        ) {
            if let Some(task_type) = task_type {
                if let Ok(semantic_cached) = self
                    .storage
                    .get_sub_agent_output_semantic(task_type, caller_agent, target_agent)
                    .await
                {
                    if !semantic_cached.is_empty() {
                        return Ok(Some(semantic_cached[0].clone()));
                    }
                }
            }
        }

        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    async fn cache_output(
        &self,
        _session_id: uuid::Uuid,
        caller_agent: &str,
        target_agent: &str,
        task_key: &str,
        task_type: &str,
        output: &str,
        cache_ttl_secs: Option<u64>,
    ) -> Result<Uuid> {
        let expires_at =
            cache_ttl_secs.map(|ttl| Utc::now() + chrono::Duration::seconds(ttl as i64));

        let output_id = Uuid::new_v4();
        let cached_output = crate::storage::SubAgentOutput {
            id: output_id,
            caller_agent: caller_agent.to_owned(),
            target_agent: target_agent.to_owned(),
            task_key: task_key.to_owned(),
            task_type: Some(task_type.to_owned()),
            output: output.to_owned(),
            created_at: Utc::now(),
            expires_at,
            metadata: json!({}),
        };

        self.storage
            .upsert_sub_agent_output(&cached_output)
            .await
            .map_err(|e| Error::Tool(format!("failed to cache output: {}", e)))?;
        Ok(output_id)
    }

    fn infer_task_type(&self, task: &str) -> String {
        let task_lower = task.to_lowercase();
        if task_lower.contains("test") || task_lower.contains("testing") {
            return "testing".to_string();
        }
        if task_lower.contains("debug") || task_lower.contains("fix bug") {
            return "debugging".to_string();
        }
        if task_lower.contains("implement") || task_lower.contains("build") {
            return "implementation".to_string();
        }
        "general".to_string()
    }

    fn apply_output_limits(&self, output: String, max_tokens: Option<usize>) -> String {
        if let Some(max_tokens) = max_tokens {
            let char_limit = max_tokens * 4;
            if output.len() > char_limit {
                let truncated = &output[..char_limit.min(output.len())];
                return format!("{}\n...[output truncated]", truncated);
            }
        }
        output
    }

    fn validate_output_schema(&self, output: &str, schema: &Value) -> Result<()> {
        let output_value: Value = serde_json::from_str(output)
            .map_err(|e| Error::Tool(format!("output is not valid JSON: {}", e)))?;

        let schema_value: Value = serde_json::from_value(schema.clone())
            .map_err(|e| Error::Tool(format!("invalid schema: {}", e)))?;

        let json_schema = jsonschema::JSONSchema::compile(&schema_value)
            .map_err(|e| Error::Tool(format!("failed to compile schema: {}", e)))?;

        let result = json_schema.validate(&output_value);
        if let Err(errors) = result {
            return Err(Error::Tool(format!(
                "output validation failed: {}",
                errors.map(|e| e.to_string()).collect::<Vec<_>>().join(", ")
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl Tool for SubAgentTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Call another configured agent with filtered session context"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let (dummy_tx, _) = mpsc::channel(1);
        self.stream_execute(args, dummy_tx, context).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let parsed = self.parse_args(args)?;
        let session_id = context
            .session_id
            .ok_or_else(|| Error::Tool("sub_agent requires an active session".to_owned()))?;
        let caller_agent = context
            .agent_name
            .as_deref()
            .ok_or_else(|| Error::Tool("sub_agent requires a caller agent context".to_owned()))?;

        let agents = {
            let guard = self
                .agents
                .read()
                .map_err(|_| Error::Tool("sub-agent registry lock poisoned".to_owned()))?;
            guard.clone()
        }
        .ok_or_else(|| Error::Tool("agent coordinator is not attached".to_owned()))?;

        let caller_config = agents.get_agent_config(caller_agent).cloned();
        let detailed_logs_enabled = caller_config
            .as_ref()
            .map(|cfg| cfg.sub_agent_parallel_detailed_logs)
            .unwrap_or(self.detailed_logs_enabled);
        let cache_mode = caller_config
            .as_ref()
            .map(|cfg| cfg.sub_agent_output_cache_mode)
            .unwrap_or(self.cache_mode);
        let cache_ttl_secs = caller_config
            .as_ref()
            .and_then(|cfg| cfg.sub_agent_output_cache_ttl_secs)
            .or(self.cache_ttl_secs);
        let auto_todos = caller_config
            .as_ref()
            .map(|cfg| cfg.auto_create_todos)
            .unwrap_or(false);

        let routing_link = self.find_latest_routing_todo_link(session_id).await;
        let routing_parent_id = routing_link.map(|(todo_id, _)| todo_id);
        let routing_trace_id = routing_link.and_then(|(_, trace_id)| trace_id);

        let todo_id = if auto_todos {
            let id = Uuid::new_v4();
            let todo = crate::storage::Todo {
                id,
                project_id: None,
                session_id,
                parent_id: routing_parent_id,
                title: format!("Sub-agent '{}' task", parsed.target_agent),
                description: Some(parsed.task.clone()),
                status: crate::storage::TodoStatus::InProgress,
                priority: crate::storage::TodoPriority::Medium,
                tags: vec!["sub-agent".to_string()],
                metadata: crate::storage::TodoMetadata {
                    tools: vec!["sub_agent".to_string()],
                    routing_trace_id,
                    ..Default::default()
                },
                created_at: Utc::now(),
                updated_at: Utc::now(),
                completed_at: None,
            };
            let _ = self.storage.create_todo(&todo).await;
            Some(id)
        } else {
            None
        };

        // Handle parallel requests if provided
        if let Some(parallel_requests) = &parsed.parallel_requests {
            if !parallel_requests.is_empty() && self.parallel_enabled {
                return self
                    .execute_parallel_requests(
                        parallel_requests,
                        session_id,
                        caller_agent,
                        tx,
                        context,
                    )
                    .await;
            }
        }

        // Check cache first if enabled
        let task_key =
            self.compute_task_key(&parsed.target_agent, &parsed.task, &parsed.context_filter);
        let inferred_task_type = self.infer_task_type(&parsed.task);

        if self.caching_enabled {
            if let Some(cached) = self
                .try_get_cached_output(
                    &task_key,
                    caller_agent,
                    &parsed.target_agent,
                    cache_mode,
                    Some(&inferred_task_type),
                )
                .await?
            {
                let _ = tx.try_send(Event::SubAgentOutputCacheHit {
                    session_id: session_id.to_string(),
                    caller_agent: caller_agent.to_owned(),
                    target_agent: parsed.target_agent.clone(),
                    task_key: task_key.clone(),
                    cache_mode: format!("{:?}", cache_mode),
                });

                let output = self.apply_output_limits(cached.output, parsed.max_output_tokens);

                if let Some(ref schema) = parsed.expected_output_schema {
                    self.validate_output_schema(&output, schema)?;
                }

                if detailed_logs_enabled {
                    let _ = tx.try_send(Event::SubAgentDetailedLog {
                        session_id: session_id.to_string(),
                        caller_agent: caller_agent.to_owned(),
                        target_agent: parsed.target_agent.clone(),
                        log_level: "info".to_string(),
                        message: format!("Using cached output for task_key: {}", task_key),
                    });
                }

                if let Some(todo_id) = todo_id {
                    let _ = self
                        .storage
                        .update_todo(
                            todo_id,
                            &crate::storage::TodoUpdate {
                                status: Some(crate::storage::TodoStatus::Completed),
                                metadata: Some(crate::storage::TodoMetadata {
                                    tools: vec!["sub_agent".to_string()],
                                    routing_trace_id,
                                    sub_agent_output_id: Some(cached.id),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        )
                        .await;
                }

                return Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output,
                });
            }
        }

        let context_filter = parsed
            .context_filter
            .map(|filter| SubAgentContextFilter {
                last_messages: filter.last_messages,
                include_roles: filter.include_roles,
                include_keywords: filter.include_keywords,
                keyword_match_mode: match filter.keyword_match_mode.as_deref() {
                    Some("all") => crate::agents::coordinator::KeywordMatchMode::All,
                    _ => crate::agents::coordinator::KeywordMatchMode::Any,
                },
                include_tool_messages: filter.include_tool_messages.unwrap_or(true),
                include_workspace: filter.include_workspace,
            })
            .unwrap_or_default();

        let _ = tx.try_send(Event::SubAgentCallStarted {
            session_id: session_id.to_string(),
            caller_agent: caller_agent.to_owned(),
            target_agent: parsed.target_agent.clone(),
            max_context_messages: context_filter.last_messages.unwrap_or(0),
        });

        let response = agents
            .run_sub_agent(
                SubAgentRequest {
                    session_id,
                    caller_agent_name: caller_agent.to_owned(),
                    target_agent_name: parsed.target_agent.clone(),
                    task: parsed.task.clone(),
                    current_depth: context.sub_agent_depth + 1,
                    context_filter,
                    max_context_tokens: parsed.max_context_tokens,
                    cancellation_token: context.cancellation_token.clone(),
                },
                tx.clone(),
            )
            .await;

        match response {
            Ok(output) => {
                let limited_output = self.apply_output_limits(output, parsed.max_output_tokens);

                // Validate output schema if provided
                if let Some(ref schema) = parsed.expected_output_schema {
                    self.validate_output_schema(&limited_output, schema)?;
                }

                // Cache the output if enabled
                let mut cached_output_id = None;
                if self.caching_enabled {
                    cached_output_id = self
                        .cache_output(
                            session_id,
                            caller_agent,
                            &parsed.target_agent,
                            &task_key,
                            &inferred_task_type,
                            &limited_output,
                            cache_ttl_secs,
                        )
                        .await
                        .ok();
                }

                if let Some(todo_id) = todo_id {
                    let _ = self
                        .storage
                        .update_todo(
                            todo_id,
                            &crate::storage::TodoUpdate {
                                status: Some(crate::storage::TodoStatus::Completed),
                                metadata: Some(crate::storage::TodoMetadata {
                                    tools: vec!["sub_agent".to_string()],
                                    routing_trace_id,
                                    sub_agent_output_id: cached_output_id,
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        )
                        .await;
                }

                let _ = tx.try_send(Event::SubAgentCallCompleted {
                    session_id: session_id.to_string(),
                    caller_agent: caller_agent.to_owned(),
                    target_agent: parsed.target_agent,
                    success: true,
                });
                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: limited_output,
                })
            }
            Err(err) => {
                if let Some(todo_id) = todo_id {
                    let _ = self
                        .storage
                        .update_todo(
                            todo_id,
                            &crate::storage::TodoUpdate {
                                status: Some(crate::storage::TodoStatus::Blocked),
                                metadata: Some(crate::storage::TodoMetadata {
                                    tools: vec!["sub_agent".to_string()],
                                    routing_trace_id,
                                    reason: Some(err.to_string()),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                let _ = tx.try_send(Event::SubAgentCallCompleted {
                    session_id: session_id.to_string(),
                    caller_agent: caller_agent.to_owned(),
                    target_agent: parsed.target_agent,
                    success: false,
                });
                Err(err)
            }
        }
    }
}
