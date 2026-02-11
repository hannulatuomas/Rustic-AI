use std::cmp::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::Duration;

use crate::config::schema::{EmbeddingBackend, RetrievalConfig};
use crate::error::{Error, Result};
use crate::storage::StorageBackend;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embedding {
    pub id: String,
    pub vector: Vec<f32>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredVector {
    pub id: String,
    pub vector: Vec<f32>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub top_k: usize,
    pub filter: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn dimension(&self) -> usize;
}

pub fn create_embedding_provider(config: &RetrievalConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.embedding_backend {
        EmbeddingBackend::DeterministicHash => Ok(Arc::new(DeterministicHashEmbedding::new(
            config.vector_dimension,
        ))),
        EmbeddingBackend::OpenAi | EmbeddingBackend::OpenAiCompatible => {
            let model = config
                .embedding_model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_owned();
            let base_url = config
                .embedding_base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_owned();
            let key_env = config
                .embedding_api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_owned();
            let key = std::env::var(&key_env).map_err(|_| {
                Error::Config(format!(
                    "embedding api key env var '{}' is not set",
                    key_env
                ))
            })?;
            Ok(Arc::new(OpenAiEmbeddingProvider::new(
                model,
                base_url,
                key,
                config.vector_dimension,
            )?))
        }
        EmbeddingBackend::SentenceTransformers => {
            let base_url = config
                .embedding_base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_owned();
            Ok(Arc::new(SentenceTransformersEmbeddingProvider::new(
                base_url,
                config.embedding_model.clone(),
                config.vector_dimension,
            )?))
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicHashEmbedding {
    dim: usize,
}

impl DeterministicHashEmbedding {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(8) }
    }
}

#[async_trait]
impl EmbeddingProvider for DeterministicHashEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut values = vec![0.0f32; self.dim];
        for (index, token) in text.split_whitespace().enumerate() {
            let hash = fxhash(token);
            let slot = (hash as usize + index) % self.dim;
            let weight = 1.0f32 + ((hash % 100) as f32 / 100.0);
            values[slot] += weight;
        }
        normalize(&mut values);
        Ok(values)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

#[derive(Clone)]
pub struct OpenAiEmbeddingProvider {
    model: String,
    endpoint: String,
    api_key: String,
    dim: usize,
    client: reqwest::Client,
}

impl OpenAiEmbeddingProvider {
    pub fn new(model: String, base_url: String, api_key: String, dim: usize) -> Result<Self> {
        let endpoint = format!("{}/embeddings", base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|err| Error::Provider(format!("failed to build embedding client: {err}")))?;
        Ok(Self {
            model,
            endpoint,
            api_key,
            dim: dim.max(8),
            client,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let response = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .json(&json!({
                "model": self.model,
                "input": text,
            }))
            .send()
            .await
            .map_err(|err| Error::Provider(format!("embedding request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_owned());
            return Err(Error::Provider(format!(
                "embedding request failed with status {status}: {body}"
            )));
        }

        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|err| Error::Provider(format!("invalid embedding response: {err}")))?;
        let vector = value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .and_then(|data| data.first())
            .and_then(|item| item.get("embedding"))
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                Error::Provider("embedding response missing data[0].embedding".to_owned())
            })?
            .iter()
            .filter_map(serde_json::Value::as_f64)
            .map(|value| value as f32)
            .collect::<Vec<_>>();

        Ok(vector)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

#[derive(Clone)]
pub struct SentenceTransformersEmbeddingProvider {
    endpoint: String,
    model: Option<String>,
    dim: usize,
    client: reqwest::Client,
}

impl SentenceTransformersEmbeddingProvider {
    pub fn new(base_url: String, model: Option<String>, dim: usize) -> Result<Self> {
        let endpoint = format!("{}/embed", base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|err| Error::Provider(format!("failed to build embedding client: {err}")))?;
        Ok(Self {
            endpoint,
            model,
            dim: dim.max(8),
            client,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for SentenceTransformersEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut payload = json!({ "text": text });
        if let Some(model) = &self.model {
            payload["model"] = json!(model);
        }

        let response = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| {
                Error::Provider(format!("sentence-transformers request failed: {err}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_owned());
            return Err(Error::Provider(format!(
                "sentence-transformers request failed with status {status}: {body}"
            )));
        }

        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|err| Error::Provider(format!("invalid embedding response: {err}")))?;
        let embedding_array = value
            .get("embedding")
            .and_then(serde_json::Value::as_array)
            .or_else(|| value.get("vector").and_then(serde_json::Value::as_array))
            .ok_or_else(|| {
                Error::Provider(
                    "sentence-transformers response missing embedding/vector array".to_owned(),
                )
            })?;

        let vector = embedding_array
            .iter()
            .filter_map(serde_json::Value::as_f64)
            .map(|value| value as f32)
            .collect::<Vec<_>>();
        Ok(vector)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

pub struct VectorDb {
    storage: Arc<dyn StorageBackend>,
    workspace: String,
}

impl VectorDb {
    pub fn new(storage: Arc<dyn StorageBackend>, workspace: String) -> Self {
        Self { storage, workspace }
    }

    pub async fn upsert_embedding(&self, embedding: &Embedding) -> Result<()> {
        self.storage
            .upsert_vector_embedding(
                &self.workspace,
                &embedding.id,
                &embedding.vector,
                &embedding.metadata,
            )
            .await
    }

    pub async fn search(&self, query_vector: &[f32], top_k: usize) -> Result<Vec<SearchResult>> {
        let vectors = self.storage.list_vector_embeddings(&self.workspace).await?;
        let mut results = vectors
            .into_iter()
            .map(|stored| SearchResult {
                id: stored.id,
                score: cosine_similarity(query_vector, &stored.vector),
                metadata: stored.metadata,
            })
            .collect::<Vec<_>>();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        results.truncate(top_k);
        Ok(results)
    }
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values.iter_mut() {
            *value /= norm;
        }
    }
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let len = left.len().min(right.len());
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for index in 0..len {
        dot += left[index] * right[index];
        left_norm += left[index] * left[index];
        right_norm += right[index] * right[index];
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn fxhash(input: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
