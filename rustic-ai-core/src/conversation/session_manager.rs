use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::config::schema::{Config, DiscoveredRuleConfig};
use crate::error::{Error, Result};
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::ChatMessage;
use crate::rules::discovery::simple_glob_match;
use crate::rules::manual_invocation::{extract_manual_invocations, resolve_manual_invocations};
use crate::rules::precedence::sort_rule_files_by_precedence;
use crate::rules::{TopicInferenceService, TopicTracker};
use crate::storage::model::{Message, PendingToolState, Session, SessionConfig};
use crate::storage::StorageBackend;

#[derive(Debug, Clone)]
pub struct LoadedRule {
    pub metadata: DiscoveredRuleConfig,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct EffectiveSummarizationConfig {
    pub enabled: bool,
    pub provider_name: Option<String>,
    pub max_context_tokens: usize,
    pub summary_max_tokens: usize,
}

pub struct SessionManager {
    storage: Arc<dyn StorageBackend>,
    discovered_rules: Vec<DiscoveredRuleConfig>,
    work_dir: PathBuf,
}

impl SessionManager {
    pub fn new(
        storage: Arc<dyn StorageBackend>,
        mut discovered_rules: Vec<DiscoveredRuleConfig>,
        work_dir: PathBuf,
    ) -> Self {
        sort_rule_files_by_precedence(&mut discovered_rules, &work_dir);
        Self {
            storage,
            discovered_rules,
            work_dir,
        }
    }

    pub async fn create_session(&self, agent_name: &str) -> Result<Uuid> {
        let session_id = Uuid::new_v4();
        self.storage
            .create_session(Session {
                id: session_id,
                agent_name: agent_name.to_owned(),
                created_at: Utc::now(),
            })
            .await?;
        Ok(session_id)
    }

    pub async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>> {
        self.storage.get_session(session_id).await
    }

    pub async fn list_sessions(&self, limit: Option<usize>) -> Result<Vec<Session>> {
        self.storage.list_sessions(limit).await
    }

    pub async fn delete_session(&self, session_id: Uuid) -> Result<()> {
        self.storage.delete_session(session_id).await
    }

    pub async fn append_message(&self, session_id: Uuid, role: &str, content: &str) -> Result<()> {
        self.storage
            .append_message(Message {
                id: Uuid::new_v4(),
                session_id,
                role: role.to_owned(),
                content: content.to_owned(),
                created_at: Utc::now(),
            })
            .await
    }

    pub async fn get_session_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.storage.get_session_messages(session_id).await
    }

    pub async fn get_session_topics(&self, session_id: Uuid) -> Result<Option<Vec<String>>> {
        self.storage.get_session_topics(session_id).await
    }

    pub async fn set_session_config(&self, session_id: Uuid, config: &SessionConfig) -> Result<()> {
        self.storage.update_session_config(session_id, config).await
    }

    pub async fn get_effective_summarization_config(
        &self,
        session_id: Uuid,
        global_config: &Config,
    ) -> Result<EffectiveSummarizationConfig> {
        let session_config = self
            .storage
            .get_session_config(session_id)
            .await?
            .unwrap_or_default();
        let project = global_config.project.as_ref();

        let enabled_base = project
            .and_then(|value| value.summarization_enabled)
            .unwrap_or(global_config.summarization.enabled);
        let provider_base = project
            .and_then(|value| value.summarization_provider_name.clone())
            .or_else(|| global_config.summarization.provider_name.clone());
        let summary_tokens_base = project
            .and_then(|value| value.summary_max_tokens)
            .unwrap_or(global_config.summarization.summary_max_tokens);

        Ok(EffectiveSummarizationConfig {
            enabled: session_config.summarization_enabled.unwrap_or(enabled_base),
            provider_name: session_config.summarization_provider_name.or(provider_base),
            max_context_tokens: global_config.summarization.max_context_tokens,
            summary_max_tokens: session_config
                .summary_max_tokens
                .unwrap_or(summary_tokens_base),
        })
    }

    pub async fn get_applicable_rules(
        &self,
        session_id: Uuid,
        current_file_path: Option<&Path>,
        input: Option<&str>,
    ) -> Result<Vec<LoadedRule>> {
        let topics = self
            .storage
            .get_session_topics(session_id)
            .await?
            .unwrap_or_default();
        let manual_invocations = input.map(extract_manual_invocations).unwrap_or_default();
        let resolved_manual =
            resolve_manual_invocations(&manual_invocations, &self.discovered_rules);

        for path in &resolved_manual {
            self.storage
                .track_manual_invocation(session_id, path)
                .await?;
        }

        let mut loaded = Vec::new();
        for rule in &self.discovered_rules {
            let manual_selected = resolved_manual.iter().any(|path| path == &rule.path);
            if !manual_selected {
                if !rule.always_apply && !rule.topics.is_empty() {
                    let topic_match = rule.topics.iter().any(|rule_topic| {
                        topics.iter().any(|active| {
                            active
                                .to_ascii_lowercase()
                                .contains(&rule_topic.to_ascii_lowercase())
                        })
                    });
                    if !topic_match {
                        continue;
                    }
                }

                if !rule.globs.is_empty() {
                    let Some(file_path) = current_file_path else {
                        continue;
                    };

                    let relative_path = file_path
                        .strip_prefix(&self.work_dir)
                        .unwrap_or(file_path)
                        .to_string_lossy()
                        .into_owned();
                    let glob_match = rule
                        .globs
                        .iter()
                        .any(|glob| simple_glob_match(glob, &relative_path));
                    if !glob_match {
                        continue;
                    }
                }
            }

            let content = self.load_rule_content(&rule.path).await?;
            loaded.push(LoadedRule {
                metadata: rule.clone(),
                content,
            });
        }

        Ok(loaded)
    }

    async fn load_rule_content(&self, path: &str) -> Result<String> {
        if let Some(content) = self.storage.get_or_load_context_file(path).await? {
            return Ok(content);
        }

        let content = std::fs::read_to_string(path)
            .map_err(|err| Error::Storage(format!("failed to read rule file '{path}': {err}")))?;
        self.storage.cache_context_file(path, &content, "").await?;
        Ok(content)
    }

    pub async fn maybe_refresh_topics(
        &self,
        session_id: Uuid,
        provider_registry: &ProviderRegistry,
        topic_inference: &TopicInferenceService,
        topic_tracker: &mut TopicTracker,
    ) -> Result<bool> {
        if !topic_tracker.should_reinfer() {
            return Ok(false);
        }

        let available_topics = self
            .discovered_rules
            .iter()
            .flat_map(|rule| rule.topics.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if available_topics.is_empty() {
            return Ok(false);
        }

        let messages = self
            .storage
            .get_recent_messages(session_id, 12)
            .await?
            .into_iter()
            .map(|message| ChatMessage {
                role: message.role,
                content: message.content,
                name: None,
                tool_calls: None,
            })
            .collect::<Vec<_>>();

        let inferred = topic_inference
            .infer_topics(provider_registry, &available_topics, &messages)
            .await?;
        if !topic_tracker.should_accept_update(&inferred) {
            return Ok(false);
        }

        topic_tracker.update_topics(inferred.clone());
        self.storage
            .update_session_topics(session_id, &inferred)
            .await?;
        Ok(true)
    }

    /// Store pending tool execution state for a session
    ///
    /// This is called when an agent tool loop encounters a tool that requires
    /// permission (Ask decision). The state is saved so that after the user
    /// approves the permission, the agent can resume exactly where it left off.
    pub async fn set_pending_tool(&self, state: PendingToolState) -> Result<()> {
        self.storage.set_pending_tool(&state).await
    }

    /// Retrieve and clear pending tool execution state for a session
    ///
    /// This is called after the user has approved a permission request.
    /// Returns None if there is no pending state.
    pub async fn get_and_clear_pending_tool(
        &self,
        session_id: Uuid,
    ) -> Result<Option<PendingToolState>> {
        self.storage.get_and_clear_pending_tool(session_id).await
    }

    /// Delete stale pending tool execution states older than the specified duration
    ///
    /// This should be called on application startup to clean up abandoned
    /// pending tool states from previous runs.
    pub async fn cleanup_stale_pending_tools(&self, older_than_secs: u64) -> Result<usize> {
        self.storage
            .delete_stale_pending_tools(older_than_secs)
            .await
    }

    /// Check if a session has pending tool execution state (without clearing it)
    pub async fn has_pending_tool(&self, session_id: Uuid) -> Result<bool> {
        self.storage.has_pending_tool(session_id).await
    }
}
