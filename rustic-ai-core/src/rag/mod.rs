use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::config::schema::{FeatureConfig, RetrievalConfig};
use crate::error::Result;
use crate::storage::StorageBackend;
use crate::vector::{create_embedding_provider, VectorDb};

#[derive(Debug, Clone)]
pub struct CodeSnippet {
    pub kind: String,
    pub file_path: String,
    pub content: String,
    pub line_start: usize,
    pub line_end: usize,
    pub score: f32,
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RetrievalResponse {
    pub snippets: Vec<CodeSnippet>,
    pub symbols: Vec<SymbolMatch>,
    pub keyword_hits: usize,
    pub vector_hits: usize,
}

#[derive(Debug, Clone)]
pub struct SymbolMatch {
    pub symbol: String,
    pub file_path: String,
    pub score: f32,
    pub usage_context: String,
}

#[derive(Debug, Clone)]
pub struct RetrievalRequest {
    pub query: String,
    pub top_k: usize,
    pub min_score: f32,
    pub filters: Option<serde_json::Value>,
}

pub struct HybridRetriever {
    storage: Arc<dyn StorageBackend>,
    workspace: PathBuf,
    features: FeatureConfig,
    config: RetrievalConfig,
}

impl HybridRetriever {
    pub fn new(
        storage: Arc<dyn StorageBackend>,
        workspace: String,
        features: FeatureConfig,
        config: RetrievalConfig,
    ) -> Self {
        Self {
            storage,
            workspace: PathBuf::from(workspace),
            features,
            config,
        }
    }

    pub async fn retrieve(&self, query: &str) -> Result<RetrievalResponse> {
        let request = RetrievalRequest {
            query: query.to_owned(),
            top_k: self.config.max_snippets,
            min_score: self.config.min_vector_score,
            filters: None,
        };
        self.retrieve_for_request(&request).await
    }

    pub async fn retrieve_for_request(
        &self,
        request: &RetrievalRequest,
    ) -> Result<RetrievalResponse> {
        if !self.features.rag_enabled || !self.config.enabled {
            return Ok(RetrievalResponse::default());
        }

        let mut snippets = Vec::new();
        let mut symbols_out = Vec::new();
        let mut seen = HashSet::new();
        let mut keyword_hits = 0usize;
        let mut vector_hits = 0usize;
        let workspace_string = self.workspace.to_string_lossy().to_string();

        if self.features.indexing_enabled && self.config.keyword_top_k > 0 {
            let symbols = self
                .storage
                .search_code_symbols(&workspace_string, &request.query, self.config.keyword_top_k)
                .await?;
            keyword_hits = symbols.len();
            for (index, symbol) in symbols.into_iter().enumerate() {
                let content = format!(
                    "symbol '{}' [{}] at {}:{}",
                    symbol.name,
                    symbol.symbol_type.as_str(),
                    symbol.file_path,
                    symbol.line
                );
                let key = format!("kw:{}:{}", symbol.file_path, symbol.name);
                if seen.insert(key) {
                    snippets.push(CodeSnippet {
                        kind: "keyword".to_owned(),
                        file_path: symbol.file_path.clone(),
                        content,
                        line_start: symbol.line,
                        line_end: symbol.line,
                        score: 1.0f32 - (index as f32 * 0.01),
                        contexts: vec!["symbol_lookup".to_owned()],
                    });
                    symbols_out.push(SymbolMatch {
                        symbol: symbol.name,
                        file_path: symbol.file_path,
                        score: 1.0f32 - (index as f32 * 0.01),
                        usage_context: "symbol_lookup".to_owned(),
                    });
                }
            }
        }

        if self.features.vector_enabled && self.config.vector_top_k > 0 {
            let embedder = create_embedding_provider(&self.config)?;
            let query_vector = embedder.embed(&request.query).await?;
            let vector_db = VectorDb::new(self.storage.clone(), workspace_string.clone());
            let hits = vector_db
                .search(&query_vector, self.config.vector_top_k)
                .await?;
            vector_hits = hits.len();
            for hit in hits {
                if hit.score < request.min_score {
                    continue;
                }
                let path = hit
                    .metadata
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<vector>")
                    .to_owned();
                let text = hit
                    .metadata
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("semantic context")
                    .to_owned();
                let key = format!("vec:{}:{}", path, hit.id);
                if seen.insert(key) {
                    snippets.push(CodeSnippet {
                        kind: "vector".to_owned(),
                        file_path: path,
                        content: text,
                        line_start: 1,
                        line_end: 1,
                        score: hit.score,
                        contexts: vec!["semantic_lookup".to_owned()],
                    });
                }
            }
        }

        snippets.retain(|snippet| self.matches_filters(snippet, request.filters.as_ref()));

        for snippet in &mut snippets {
            self.expand_context(snippet);
            self.apply_ranking_adjustments(snippet);
        }

        snippets.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
        });
        if snippets.len() > request.top_k {
            snippets.truncate(request.top_k);
        }
        for snippet in &mut snippets {
            if snippet.content.chars().count() > self.config.max_snippet_chars {
                snippet.content = snippet
                    .content
                    .chars()
                    .take(self.config.max_snippet_chars)
                    .collect::<String>();
            }
        }

        Ok(RetrievalResponse {
            snippets,
            symbols: symbols_out,
            keyword_hits,
            vector_hits,
        })
    }

    pub fn format_for_prompt(&self, snippets: &[CodeSnippet]) -> String {
        if snippets.is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();
        let mut used_tokens = 0usize;
        let budget = self.config.rag_prompt_token_budget.max(64);
        lines.push("Retrieved context from code index:".to_owned());
        used_tokens += lines[0].chars().count() / 4;
        for snippet in snippets {
            let rendered = format!(
                "- [{}] {}:{}-{} :: {}",
                snippet.kind,
                snippet.file_path,
                snippet.line_start,
                snippet.line_end,
                snippet.content
            );
            let estimated = std::cmp::max(1, rendered.chars().count() / 4);
            if used_tokens + estimated > budget {
                break;
            }
            used_tokens += estimated;
            lines.push(rendered);
        }
        lines.join("\n")
    }

    pub fn inject_as_system_message(&self) -> bool {
        self.config.inject_as_system_message
    }

    fn matches_filters(&self, snippet: &CodeSnippet, filters: Option<&serde_json::Value>) -> bool {
        let Some(filters) = filters else {
            return true;
        };

        if let Some(path_prefix) = filters
            .get("path_prefix")
            .and_then(serde_json::Value::as_str)
        {
            if !snippet.file_path.starts_with(path_prefix) {
                return false;
            }
        }
        if let Some(kind) = filters.get("kind").and_then(serde_json::Value::as_str) {
            if snippet.kind != kind {
                return false;
            }
        }

        true
    }

    fn expand_context(&self, snippet: &mut CodeSnippet) {
        if self.config.context_expansion_lines == 0 {
            return;
        }
        let resolved_path = self.workspace.join(&snippet.file_path);
        let Ok(content) = std::fs::read_to_string(&resolved_path) else {
            return;
        };
        let lines = content.lines().collect::<Vec<_>>();
        if lines.is_empty() {
            return;
        }

        let center_line = snippet.line_start.max(1);
        let start = center_line
            .saturating_sub(self.config.context_expansion_lines)
            .max(1);
        let end = (center_line + self.config.context_expansion_lines).min(lines.len());
        if start > end || end == 0 {
            return;
        }

        let mut expanded = Vec::new();
        for number in start..=end {
            if let Some(line) = lines.get(number - 1) {
                expanded.push(format!("{:>4}: {}", number, line.trim_end()));
            }
        }
        if !expanded.is_empty() {
            snippet.line_start = start;
            snippet.line_end = end;
            snippet.content = expanded.join(" | ");
            if !snippet
                .contexts
                .iter()
                .any(|entry| entry == "expanded_context")
            {
                snippet.contexts.push("expanded_context".to_owned());
            }
        }
    }

    fn apply_ranking_adjustments(&self, snippet: &mut CodeSnippet) {
        let resolved_path = self.workspace.join(&snippet.file_path);
        let recency_score = file_recency_score(&resolved_path);
        let importance_score = if snippet.kind == "keyword" { 1.0 } else { 0.75 };

        snippet.score += recency_score * self.config.ranking_recency_weight;
        snippet.score += importance_score * self.config.ranking_importance_weight;
    }
}

fn file_recency_score(path: &PathBuf) -> f32 {
    let Ok(metadata) = std::fs::metadata(path) else {
        return 0.35;
    };
    let Ok(modified) = metadata.modified() else {
        return 0.35;
    };
    let now = SystemTime::now();
    let Ok(age) = now.duration_since(modified) else {
        return 0.5;
    };
    let age_hours = age.as_secs_f32() / 3600.0;
    let age_days = age_hours / 24.0;
    1.0 / (1.0 + age_days)
}
