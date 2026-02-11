use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use reqwest::Url;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const DEFAULT_NUM_RESULTS: usize = 8;
const MAX_NUM_RESULTS: usize = 25;

#[derive(Debug, Clone)]
pub struct WebSearchTool {
    config: ToolConfig,
    schema: Value,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchEngine {
    Google,
    Bing,
    DuckDuckGo,
    Auto,
}

#[derive(Debug, Clone)]
struct WebSearchArgs {
    query: String,
    num_results: usize,
    engine: SearchEngine,
    lang: String,
    timeout_seconds: u64,
    google_api_key: Option<String>,
    google_cx: Option<String>,
    bing_api_key: Option<String>,
}

#[derive(Debug, Clone)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
    source_engine: String,
    score: f64,
}

impl SearchEngine {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "google" => Ok(Self::Google),
            "bing" => Ok(Self::Bing),
            "duckduckgo" | "ddg" => Ok(Self::DuckDuckGo),
            "auto" => Ok(Self::Auto),
            other => Err(Error::Tool(format!(
                "unsupported search engine '{other}' (expected auto|google|bing|duckduckgo)"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Bing => "bing",
            Self::DuckDuckGo => "duckduckgo",
            Self::Auto => "auto",
        }
    }
}

impl WebSearchTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "num_results": { "type": "integer", "minimum": 1, "maximum": 25 },
                "engine": { "type": "string", "enum": ["auto", "google", "bing", "duckduckgo"] },
                "lang": { "type": "string" },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 120 },
                "google_api_key": { "type": "string" },
                "google_cx": { "type": "string" },
                "bing_api_key": { "type": "string" }
            },
            "required": ["query"]
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Rustic-AI-WebSearch/0.1 (+https://github.com)"),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html,application/json"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            config,
            schema,
            client,
        }
    }

    fn parse_args(&self, args: &Value) -> Result<WebSearchArgs> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Tool("missing 'query' argument".to_owned()))?
            .to_owned();
        let num_results = args
            .get("num_results")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_NUM_RESULTS as u64)
            .clamp(1, MAX_NUM_RESULTS as u64) as usize;
        let engine =
            SearchEngine::parse(args.get("engine").and_then(Value::as_str).unwrap_or("auto"))?;
        let lang = args
            .get("lang")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("en")
            .to_owned();
        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 120);

        let google_api_key = args
            .get("google_api_key")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("GOOGLE_SEARCH_API_KEY").ok());
        let google_cx = args
            .get("google_cx")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("GOOGLE_SEARCH_CX").ok());
        let bing_api_key = args
            .get("bing_api_key")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("BING_SEARCH_API_KEY").ok());

        Ok(WebSearchArgs {
            query,
            num_results,
            engine,
            lang,
            timeout_seconds,
            google_api_key,
            google_cx,
            bing_api_key,
        })
    }

    async fn run_with_controls<F, T>(
        &self,
        timeout_seconds: u64,
        cancellation_token: Option<tokio_util::sync::CancellationToken>,
        operation: F,
    ) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        if let Some(token) = cancellation_token {
            tokio::select! {
                _ = token.cancelled() => Err(Error::Timeout("web search cancelled".to_owned())),
                result = timeout(Duration::from_secs(timeout_seconds), operation) => {
                    match result {
                        Ok(inner) => inner,
                        Err(_) => Err(Error::Timeout(format!("web search timed out after {timeout_seconds} seconds"))),
                    }
                }
            }
        } else {
            match timeout(Duration::from_secs(timeout_seconds), operation).await {
                Ok(inner) => inner,
                Err(_) => Err(Error::Timeout(format!(
                    "web search timed out after {timeout_seconds} seconds"
                ))),
            }
        }
    }

    fn html_unescape(input: &str) -> String {
        input
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&nbsp;", " ")
    }

    fn strip_html(input: &str) -> String {
        let mut text = String::with_capacity(input.len());
        let mut in_tag = false;
        for ch in input.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => text.push(ch),
                _ => {}
            }
        }
        Self::html_unescape(text.trim())
    }

    fn parse_duckduckgo_html(&self, html: &str, limit: usize) -> Vec<SearchResult> {
        let Ok(item_re) = regex::Regex::new(
            r#"(?s)<a[^>]*class=\"[^\"]*result__a[^\"]*\"[^>]*href=\"([^\"]+)\"[^>]*>(.*?)</a>"#,
        ) else {
            return Vec::new();
        };
        let Ok(snippet_re) = regex::Regex::new(
            r#"(?s)<a[^>]*class=\"[^\"]*result__a[^\"]*\"[^>]*>.*?</a>\s*.*?<a[^>]*class=\"[^\"]*result__snippet[^\"]*\"[^>]*>(.*?)</a>"#,
        ) else {
            return Vec::new();
        };

        let mut results = Vec::new();
        let snippets = snippet_re
            .captures_iter(html)
            .filter_map(|cap| cap.get(1).map(|m| Self::strip_html(m.as_str())))
            .collect::<Vec<_>>();

        for (idx, cap) in item_re.captures_iter(html).enumerate() {
            let Some(url) = cap.get(1).map(|m| m.as_str().trim()) else {
                continue;
            };
            let title = cap
                .get(2)
                .map(|m| Self::strip_html(m.as_str()))
                .unwrap_or_default();
            if title.is_empty() || url.is_empty() {
                continue;
            }
            let snippet = snippets.get(idx).cloned().unwrap_or_default();
            results.push(SearchResult {
                title,
                url: url.to_owned(),
                snippet,
                source_engine: "duckduckgo".to_owned(),
                score: 0.0,
            });
            if results.len() >= limit {
                break;
            }
        }

        results
    }

    async fn search_duckduckgo_html(&self, args: &WebSearchArgs) -> Result<Vec<SearchResult>> {
        let mut url = Url::parse("https://duckduckgo.com/html/")
            .map_err(|err| Error::Tool(format!("failed to build duckduckgo URL: {err}")))?;
        url.query_pairs_mut()
            .append_pair("q", &args.query)
            .append_pair("kl", &format!("{}-{}", args.lang, args.lang));

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|err| Error::Tool(format!("duckduckgo search request failed: {err}")))?;
        if !response.status().is_success() {
            return Err(Error::Tool(format!(
                "duckduckgo search failed with status {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|err| Error::Tool(format!("failed to read duckduckgo response: {err}")))?;
        Ok(self.parse_duckduckgo_html(&body, args.num_results))
    }

    async fn search_bing_api(&self, args: &WebSearchArgs) -> Result<Vec<SearchResult>> {
        let key = args
            .bing_api_key
            .clone()
            .ok_or_else(|| Error::Tool("bing_api_key is required for engine='bing'".to_owned()))?;

        let mut url = Url::parse("https://api.bing.microsoft.com/v7.0/search")
            .map_err(|err| Error::Tool(format!("failed to build bing URL: {err}")))?;
        url.query_pairs_mut()
            .append_pair("q", &args.query)
            .append_pair("count", &args.num_results.to_string())
            .append_pair("mkt", &format!("{}-{}", args.lang, args.lang));

        let response = self
            .client
            .get(url)
            .header("Ocp-Apim-Subscription-Key", key)
            .send()
            .await
            .map_err(|err| Error::Tool(format!("bing search request failed: {err}")))?;
        if !response.status().is_success() {
            return Err(Error::Tool(format!(
                "bing search failed with status {}",
                response.status()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|err| Error::Tool(format!("invalid bing JSON response: {err}")))?;

        let mut results = Vec::new();
        if let Some(items) = payload
            .get("webPages")
            .and_then(|web_pages| web_pages.get("value"))
            .and_then(Value::as_array)
        {
            for item in items {
                let title = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let url = item.get("url").and_then(Value::as_str).unwrap_or_default();
                let snippet = item
                    .get("snippet")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if title.is_empty() || url.is_empty() {
                    continue;
                }
                results.push(SearchResult {
                    title: title.to_owned(),
                    url: url.to_owned(),
                    snippet: snippet.to_owned(),
                    source_engine: "bing".to_owned(),
                    score: 0.0,
                });
            }
        }

        Ok(results)
    }

    async fn search_google_api(&self, args: &WebSearchArgs) -> Result<Vec<SearchResult>> {
        let key = args.google_api_key.clone().ok_or_else(|| {
            Error::Tool("google_api_key is required for engine='google'".to_owned())
        })?;
        let cx = args
            .google_cx
            .clone()
            .ok_or_else(|| Error::Tool("google_cx is required for engine='google'".to_owned()))?;

        let mut url = Url::parse("https://www.googleapis.com/customsearch/v1")
            .map_err(|err| Error::Tool(format!("failed to build google URL: {err}")))?;
        url.query_pairs_mut()
            .append_pair("q", &args.query)
            .append_pair("key", &key)
            .append_pair("cx", &cx)
            .append_pair("num", &args.num_results.min(10).to_string())
            .append_pair("lr", &format!("lang_{}", args.lang));

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|err| Error::Tool(format!("google search request failed: {err}")))?;
        if !response.status().is_success() {
            return Err(Error::Tool(format!(
                "google search failed with status {}",
                response.status()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|err| Error::Tool(format!("invalid google JSON response: {err}")))?;

        let mut results = Vec::new();
        if let Some(items) = payload.get("items").and_then(Value::as_array) {
            for item in items {
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let url = item.get("link").and_then(Value::as_str).unwrap_or_default();
                let snippet = item
                    .get("snippet")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if title.is_empty() || url.is_empty() {
                    continue;
                }
                results.push(SearchResult {
                    title: title.to_owned(),
                    url: url.to_owned(),
                    snippet: snippet.to_owned(),
                    source_engine: "google".to_owned(),
                    score: 0.0,
                });
            }
        }

        Ok(results)
    }

    fn score_results(&self, query: &str, results: &mut [SearchResult]) {
        let tokens = query
            .split_whitespace()
            .map(|t| t.to_ascii_lowercase())
            .collect::<Vec<_>>();

        for result in results.iter_mut() {
            let title = result.title.to_ascii_lowercase();
            let snippet = result.snippet.to_ascii_lowercase();
            let url = result.url.to_ascii_lowercase();

            let mut score = 0.0;
            for token in &tokens {
                if title.contains(token) {
                    score += 3.0;
                }
                if snippet.contains(token) {
                    score += 1.5;
                }
                if url.contains(token) {
                    score += 0.5;
                }
            }
            score += (1.0 / (result.url.len() as f64 + 1.0)) * 10.0;
            result.score = score;
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    fn dedupe_results(&self, results: Vec<SearchResult>) -> Vec<SearchResult> {
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for result in results {
            let key = result.url.to_ascii_lowercase();
            if seen.insert(key) {
                deduped.push(result);
            }
        }
        deduped
    }

    async fn search_once(
        &self,
        engine: SearchEngine,
        args: &WebSearchArgs,
    ) -> Result<Vec<SearchResult>> {
        match engine {
            SearchEngine::DuckDuckGo => self.search_duckduckgo_html(args).await,
            SearchEngine::Bing => self.search_bing_api(args).await,
            SearchEngine::Google => self.search_google_api(args).await,
            SearchEngine::Auto => Err(Error::Tool(
                "internal error: auto engine should be resolved before search_once".to_owned(),
            )),
        }
    }

    fn auto_engine_order(&self, args: &WebSearchArgs) -> Vec<SearchEngine> {
        let mut order = Vec::new();
        if args.bing_api_key.is_some() {
            order.push(SearchEngine::Bing);
        }
        if args.google_api_key.is_some() && args.google_cx.is_some() {
            order.push(SearchEngine::Google);
        }
        order.push(SearchEngine::DuckDuckGo);
        order
    }

    async fn execute_search(
        &self,
        args: WebSearchArgs,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<Value> {
        let engine_order = if args.engine == SearchEngine::Auto {
            self.auto_engine_order(&args)
        } else {
            vec![args.engine]
        };

        let mut per_engine_errors = HashMap::new();
        let mut collected = Vec::new();
        let mut used_engine = "".to_owned();

        for engine in engine_order {
            if let Some(tx) = tx.as_ref() {
                let _ = tx.try_send(Event::ToolOutput {
                    tool: self.config.name.clone(),
                    stdout_chunk: format!("searching with engine '{}'\n", engine.as_str()),
                    stderr_chunk: String::new(),
                });
            }

            match self.search_once(engine, &args).await {
                Ok(results) if !results.is_empty() => {
                    collected = results;
                    used_engine = engine.as_str().to_owned();
                    break;
                }
                Ok(_) => {
                    per_engine_errors
                        .insert(engine.as_str().to_owned(), "no results returned".to_owned());
                }
                Err(err) => {
                    per_engine_errors.insert(engine.as_str().to_owned(), err.to_string());
                }
            }
        }

        if collected.is_empty() {
            return Err(Error::Tool(format!(
                "all web search engines failed or returned no results: {}",
                serde_json::to_string(&per_engine_errors).unwrap_or_default()
            )));
        }

        let mut ranked = self.dedupe_results(collected);
        self.score_results(&args.query, &mut ranked);
        ranked.truncate(args.num_results);

        let items = ranked
            .iter()
            .enumerate()
            .map(|(index, item)| {
                json!({
                    "rank": index + 1,
                    "title": item.title,
                    "url": item.url,
                    "snippet": item.snippet,
                    "source_engine": item.source_engine,
                    "score": item.score,
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "query": args.query,
            "requested_engine": args.engine.as_str(),
            "used_engine": used_engine,
            "count": items.len(),
            "results": items,
            "engine_errors": per_engine_errors,
        }))
    }

    async fn execute_operation(
        &self,
        args: Value,
        context: &ToolExecutionContext,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<ToolResult> {
        let parsed = self.parse_args(&args)?;
        let payload = self
            .run_with_controls(
                parsed.timeout_seconds,
                context.cancellation_token.clone(),
                self.execute_search(parsed, tx),
            )
            .await?;

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Search web results via APIs and HTML fallbacks"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        self.execute_operation(args, context, None).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = self.name().to_owned();
        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let result = self
            .execute_operation(args, context, Some(tx.clone()))
            .await;
        match &result {
            Ok(tool_result) => {
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("web_search tool failed: {err}")));
            }
        }

        result
    }
}
