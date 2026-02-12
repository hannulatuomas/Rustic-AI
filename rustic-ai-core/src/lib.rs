pub mod agents;
pub mod auth;
pub mod catalog;
pub mod commands;
pub mod config;
pub mod conversation;
pub mod error;
pub mod events;
pub mod indexing;
pub mod learning;
pub mod logging;
pub mod permissions;
pub mod project;
pub mod providers;
pub mod rag;
pub mod routing;
pub mod rules;
pub mod runtime;
pub mod skills;
pub mod storage;
pub mod tools;
pub mod vector;
pub mod workflows;

pub use agents::Agent;
pub use auth::CredentialStore;
pub use config::schema::PermissionConfig;
pub use config::Config;
pub use conversation::session_manager::SessionManager;
pub use error::{Error, Result};
pub use events::EventBus;
pub use indexing::CodeIndexer;
pub use indexing::{CodeGraph, ImpactReport};
pub use learning::types::{
    FeedbackContext, FeedbackType, MistakeType, PatternCategory, PreferenceValue,
};
pub use learning::LearningManager;
pub use providers::create_provider_registry;
pub use rag::HybridRetriever;
pub use storage::create_storage_backend;
pub use tools::ToolManager;
pub use vector::{DeterministicHashEmbedding, Embedding, SearchQuery, SearchResult, VectorDb};

pub struct RusticAI {
    config: Config,
    runtime: runtime::Runtime,
    session_manager: std::sync::Arc<conversation::session_manager::SessionManager>,
    learning: std::sync::Arc<learning::LearningManager>,
    topic_inference: rules::TopicInferenceService,
    work_dir: std::path::PathBuf,
}

impl RusticAI {
    pub fn new(mut config: Config) -> Result<Self> {
        config::validate_config(&config)?;
        let work_dir = std::env::current_dir()
            .map_err(|err| Error::Config(format!("failed to read current directory: {err}")))?;
        let storage_paths = storage::paths::StoragePaths::resolve(&work_dir, &config);
        std::fs::create_dir_all(&storage_paths.project_data_dir)?;
        std::fs::create_dir_all(&storage_paths.global_data_dir)?;
        if let Some(parent) = storage_paths.global_settings.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !storage_paths.global_settings.exists() {
            std::fs::write(&storage_paths.global_settings, "{}")?;
        }

        // Set default permissions if not present
        if config.permissions.default_tool_permission == crate::config::schema::PermissionMode::Ask
            && config.permissions.ask_decisions_persist_scope
                == crate::config::schema::DecisionScope::Session
        {
            // Use default values
            config.permissions = crate::config::schema::PermissionConfig::default();
        }

        let storage_backend = storage::create_storage_backend(&config, &storage_paths)?;
        let project_profile = if config.mode == crate::config::schema::RuntimeMode::Project {
            config
                .project
                .as_ref()
                .map(|project| crate::project::profile::ProjectProfile {
                    name: project.name.clone(),
                    root_path: project.root_path.clone(),
                    tech_stack: project.tech_stack.clone(),
                    goals: project.goals.clone(),
                    preferences: project.preferences.clone(),
                    style_guidelines: project.style_guidelines.clone(),
                })
        } else {
            None
        };
        let session_manager =
            std::sync::Arc::new(conversation::session_manager::SessionManager::new(
                storage_backend.clone(),
                config.rules.discovered_rules.clone(),
                work_dir.clone(),
                project_profile,
            ));
        let learning = std::sync::Arc::new(learning::LearningManager::new(
            storage_backend.clone(),
            config.features.learning_enabled,
        ));
        let retriever = std::sync::Arc::new(rag::HybridRetriever::new(
            storage_backend,
            work_dir.to_string_lossy().to_string(),
            config.features.clone(),
            config.retrieval.clone(),
        ));

        // Cleanup stale pending tool states on startup using a dedicated runtime
        let cleanup_timeout = config.permissions.pending_tool_timeout_secs;
        let session_manager_clone = session_manager.clone();
        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Runtime::new() {
                let _ =
                    rt.block_on(session_manager_clone.cleanup_stale_pending_tools(cleanup_timeout));
            }
        });

        let runtime = runtime::Runtime::new(
            config.clone(),
            session_manager.clone(),
            learning.clone(),
            retriever,
        )?;
        let inference_provider = config.summarization.provider_name.clone().ok_or_else(|| {
            Error::Config(
                "summarization.provider_name must be set (no implicit provider fallback)"
                    .to_owned(),
            )
        })?;
        let topic_inference = rules::TopicInferenceService::new(inference_provider);

        Ok(Self {
            config,
            runtime,
            session_manager,
            learning,
            topic_inference,
            work_dir,
        })
    }

    pub fn from_config_path(path: &std::path::Path) -> Result<Self> {
        let config = config::load(Some(path))?;
        Self::new(config)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn runtime(&self) -> &runtime::Runtime {
        &self.runtime
    }

    pub fn session_manager(
        &self,
    ) -> &std::sync::Arc<conversation::session_manager::SessionManager> {
        &self.session_manager
    }

    pub fn topic_inference(&self) -> &rules::TopicInferenceService {
        &self.topic_inference
    }

    pub fn learning(&self) -> &std::sync::Arc<learning::LearningManager> {
        &self.learning
    }

    pub fn work_dir(&self) -> &std::path::Path {
        &self.work_dir
    }

    pub fn code_indexer(&self) -> indexing::CodeIndexer {
        indexing::CodeIndexer::new(
            self.session_manager.storage(),
            self.work_dir.clone(),
            self.config.features.indexing_enabled,
            self.config.features.vector_enabled,
            self.config.retrieval.clone(),
        )
    }

    pub async fn build_code_index(&self) -> Result<indexing::CodeIndex> {
        self.code_indexer().build_index().await
    }

    pub async fn search_code_symbols(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<indexing::SymbolIndex>> {
        self.code_indexer().search_symbols(query, limit).await
    }

    pub async fn load_code_index_snapshot(&self) -> Result<indexing::CodeIndex> {
        self.code_indexer().load_index_snapshot().await
    }

    pub async fn build_code_graph(&self) -> Result<indexing::CodeGraph> {
        let snapshot = self.load_code_index_snapshot().await?;
        Ok(indexing::build_code_graph(&snapshot))
    }

    pub async fn analyze_symbol_impact(
        &self,
        symbol: &str,
        depth: usize,
    ) -> Result<indexing::ImpactReport> {
        let snapshot = self.load_code_index_snapshot().await?;
        Ok(indexing::analyze_impact(&snapshot, symbol, depth))
    }

    pub async fn render_code_graph_dot(&self) -> Result<String> {
        let graph = self.build_code_graph().await?;
        Ok(indexing::render_dot(&graph))
    }

    pub async fn retrieve_code_context(
        &self,
        query: &str,
        top_k: usize,
        min_score: Option<f32>,
        filters: Option<serde_json::Value>,
    ) -> Result<rag::RetrievalResponse> {
        let retriever = rag::HybridRetriever::new(
            self.session_manager.storage(),
            self.work_dir.to_string_lossy().to_string(),
            self.config.features.clone(),
            self.config.retrieval.clone(),
        );
        let request = rag::RetrievalRequest {
            query: query.to_owned(),
            top_k,
            min_score: min_score.unwrap_or(self.config.retrieval.min_vector_score),
            filters,
        };
        retriever.retrieve_for_request(&request).await
    }
}
