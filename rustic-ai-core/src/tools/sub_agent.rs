use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::agents::coordinator::{AgentCoordinator, SubAgentContextFilter, SubAgentRequest};
use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct SubAgentArgs {
    target_agent: String,
    task: String,
    context_filter: Option<SubAgentContextFilterArgs>,
    max_context_tokens: Option<usize>,
}

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

#[derive(Clone)]
pub struct SubAgentTool {
    config: ToolConfig,
    schema: Value,
    agents: Arc<StdRwLock<Option<Arc<AgentCoordinator>>>>,
}

impl SubAgentTool {
    pub fn new(config: ToolConfig, agents: Arc<StdRwLock<Option<Arc<AgentCoordinator>>>>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "target_agent": {
                    "type": "string",
                    "description": "Configured target agent name"
                },
                "task": {
                    "type": "string",
                    "description": "Task sent to the target agent"
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
                }
            },
            "required": ["target_agent", "task"]
        });

        Self {
            config,
            schema,
            agents,
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
                },
                tx.clone(),
            )
            .await;

        match response {
            Ok(output) => {
                let _ = tx.try_send(Event::SubAgentCallCompleted {
                    session_id: session_id.to_string(),
                    caller_agent: caller_agent.to_owned(),
                    target_agent: parsed.target_agent,
                    success: true,
                });
                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output,
                })
            }
            Err(err) => {
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
