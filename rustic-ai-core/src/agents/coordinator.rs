use crate::agents::behavior::Agent;
use crate::agents::registry::{AgentRegistry, AgentSuggestion};
use crate::config::schema::AgentConfig;
use crate::conversation::session_manager::SessionManager;
use crate::error::{Error, Result};
use crate::learning::LearningManager;
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::ChatMessage;
use crate::rag::HybridRetriever;
use crate::ToolManager;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const DEFAULT_SUB_AGENT_CONTEXT_MESSAGES: usize = 24;
const DEFAULT_SUB_AGENT_MAX_CONTEXT_TOKENS: usize = 4_000;

#[derive(Debug, Clone)]
pub struct SubAgentRequest {
    pub session_id: Uuid,
    pub caller_agent_name: String,
    pub target_agent_name: String,
    pub task: String,
    pub current_depth: usize,
    pub context_filter: SubAgentContextFilter,
    pub max_context_tokens: Option<usize>,
    pub cancellation_token: Option<CancellationToken>,
}

#[derive(Debug, Clone)]
pub struct SubAgentContextFilter {
    pub last_messages: Option<usize>,
    pub include_roles: Option<Vec<String>>,
    pub include_keywords: Option<Vec<String>>,
    pub keyword_match_mode: KeywordMatchMode,
    pub include_tool_messages: bool,
    pub include_workspace: bool,
}

impl Default for SubAgentContextFilter {
    fn default() -> Self {
        Self {
            last_messages: None,
            include_roles: None,
            include_keywords: None,
            keyword_match_mode: KeywordMatchMode::Any,
            include_tool_messages: true,
            include_workspace: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum KeywordMatchMode {
    #[default]
    Any,
    All,
}

#[derive(Clone)]
pub struct AgentCoordinator {
    registry: Arc<AgentRegistry>,
    default_agent: String,
    session_manager: Arc<SessionManager>,
}

impl AgentCoordinator {
    pub fn new(
        agent_configs: Vec<AgentConfig>,
        provider_registry: &ProviderRegistry,
        tool_manager: Arc<ToolManager>,
        session_manager: Arc<SessionManager>,
        learning: Arc<LearningManager>,
        retriever: Arc<HybridRetriever>,
    ) -> Result<Self> {
        let mut registry = AgentRegistry::new();
        let mut default_agent = String::new();

        for config in agent_configs {
            let provider = provider_registry
                .get_provider(&config.provider)
                .ok_or_else(|| {
                    Error::Config(format!(
                        "agent '{}' references unknown provider '{}'",
                        config.name, config.provider
                    ))
                })?;

            let agent = Arc::new(Agent::new(
                config.clone(),
                provider,
                tool_manager.clone(),
                session_manager.clone(),
                learning.clone(),
                retriever.clone(),
            ));

            registry.register(config.clone(), agent);

            // Use first agent as default if not specified
            if default_agent.is_empty() {
                default_agent = config.name.clone();
            }
        }

        Ok(Self {
            registry: Arc::new(registry),
            default_agent,
            session_manager,
        })
    }

    pub fn get_agent(&self, name: Option<&str>) -> Result<Arc<Agent>> {
        let name = name.unwrap_or(&self.default_agent);
        self.registry
            .get_agent(name)
            .ok_or_else(|| Error::NotFound(format!("agent '{}' not found", name)))
    }

    pub fn get_agent_config(&self, name: &str) -> Option<&AgentConfig> {
        self.registry.get_config(name)
    }

    pub fn has_agent(&self, name: &str) -> bool {
        self.registry.has_agent(name)
    }

    pub fn list_agents(&self) -> Vec<String> {
        self.registry.list_agents()
    }

    pub fn find_agents_by_tool(&self, tool_name: &str) -> Vec<String> {
        self.registry.find_by_tool(tool_name)
    }

    pub fn find_agents_by_skill(&self, skill_name: &str) -> Vec<String> {
        self.registry.find_by_skill(skill_name)
    }

    pub fn suggest_agents_for_task(&self, task_description: &str) -> Vec<AgentSuggestion> {
        self.registry.suggest_for_task(task_description)
    }

    pub async fn run_sub_agent(
        &self,
        request: SubAgentRequest,
        event_tx: mpsc::Sender<crate::events::Event>,
    ) -> Result<String> {
        let caller_config = self
            .get_agent_config(&request.caller_agent_name)
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "caller agent '{}' not found",
                    request.caller_agent_name
                ))
            })?;
        if !caller_config.allow_sub_agent_calls {
            return Err(Error::Tool(format!(
                "agent '{}' is not allowed to call sub-agents",
                request.caller_agent_name
            )));
        }

        let max_depth = caller_config.max_sub_agent_depth.unwrap_or(3);
        if request.current_depth >= max_depth {
            return Err(Error::Tool(format!(
                "sub-agent depth {} reached configured max_sub_agent_depth {} for agent '{}'",
                request.current_depth, max_depth, request.caller_agent_name
            )));
        }

        if request.caller_agent_name == request.target_agent_name {
            return Err(Error::Validation(
                "agent cannot call itself as sub-agent".to_owned(),
            ));
        }

        let messages = self
            .build_sub_agent_context(
                request.session_id,
                &request.context_filter,
                request.max_context_tokens.unwrap_or_else(|| {
                    caller_config
                        .sub_agent_max_context_tokens
                        .unwrap_or(DEFAULT_SUB_AGENT_MAX_CONTEXT_TOKENS)
                }),
                caller_config
                    .sub_agent_context_window_size
                    .unwrap_or(DEFAULT_SUB_AGENT_CONTEXT_MESSAGES),
            )
            .await?;
        let target_agent = self.get_agent(Some(&request.target_agent_name))?;
        target_agent
            .generate_from_context(
                messages,
                &request.task,
                request.session_id.to_string(),
                event_tx,
                request.cancellation_token,
            )
            .await
    }

    async fn build_sub_agent_context(
        &self,
        session_id: Uuid,
        filter: &SubAgentContextFilter,
        max_context_tokens: usize,
        default_context_messages: usize,
    ) -> Result<Vec<ChatMessage>> {
        let mut messages = self
            .session_manager
            .get_session_messages(session_id)
            .await?;

        if let Some(include_roles) = &filter.include_roles {
            let normalized_roles = include_roles
                .iter()
                .map(|role| role.trim().to_ascii_lowercase())
                .collect::<Vec<_>>();
            messages.retain(|message| {
                normalized_roles
                    .iter()
                    .any(|role| role == &message.role.to_ascii_lowercase())
            });
        }

        if let Some(include_keywords) = &filter.include_keywords {
            let normalized = include_keywords
                .iter()
                .map(|keyword| keyword.trim().to_ascii_lowercase())
                .filter(|keyword| !keyword.is_empty())
                .collect::<Vec<_>>();
            if !normalized.is_empty() {
                messages.retain(|message| {
                    let content = message.content.to_ascii_lowercase();
                    match filter.keyword_match_mode {
                        KeywordMatchMode::Any => {
                            normalized.iter().any(|keyword| content.contains(keyword))
                        }
                        KeywordMatchMode::All => {
                            normalized.iter().all(|keyword| content.contains(keyword))
                        }
                    }
                });
            }
        }

        if !filter.include_tool_messages {
            messages.retain(|message| message.role != "tool");
        }

        let total = messages.len();
        let last_messages = filter.last_messages.unwrap_or(default_context_messages);
        let start = total.saturating_sub(last_messages);
        let mut selected = messages
            .into_iter()
            .skip(start)
            .map(|message| ChatMessage {
                role: message.role,
                content: message.content,
                name: None,
                tool_calls: None,
            })
            .collect::<Vec<_>>();

        if filter.include_workspace {
            selected.insert(0, Self::workspace_summary_message());
        }

        selected = Self::trim_to_token_budget(selected, max_context_tokens);

        Ok(selected)
    }

    fn workspace_summary_message() -> ChatMessage {
        let cwd = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "<unknown>".to_owned());
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&cwd) {
            for entry in read_dir.flatten().take(12) {
                entries.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        let listing = if entries.is_empty() {
            "<unavailable>".to_owned()
        } else {
            entries.join(", ")
        };

        ChatMessage {
            role: "system".to_owned(),
            content: format!(
                "Workspace context:\n- cwd: {}\n- top-level entries: {}",
                cwd, listing
            ),
            name: None,
            tool_calls: None,
        }
    }

    fn trim_to_token_budget(messages: Vec<ChatMessage>, token_budget: usize) -> Vec<ChatMessage> {
        if token_budget == 0 {
            return messages;
        }

        let mut selected = Vec::new();
        let mut used_tokens = 0usize;

        for message in messages.into_iter().rev() {
            let estimated_tokens = std::cmp::max(1, message.content.chars().count() / 4)
                + std::cmp::max(1, message.role.len() / 4);
            if used_tokens + estimated_tokens > token_budget {
                continue;
            }
            used_tokens += estimated_tokens;
            selected.push(message);
        }

        selected.reverse();
        selected
    }
}
