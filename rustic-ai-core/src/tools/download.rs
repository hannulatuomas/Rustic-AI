use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const MAX_DOWNLOAD_BYTES_HARD: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct DownloadTool {
    config: ToolConfig,
    schema: Value,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct DownloadArgs {
    url: String,
    output: String,
    resume: bool,
    chunk_size: usize,
    max_size: u64,
    timeout_seconds: u64,
    sha256: Option<String>,
}

impl DownloadTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "output": { "type": "string" },
                "resume": { "type": "boolean" },
                "chunk_size": { "type": "integer", "minimum": 1024, "maximum": 4194304 },
                "max_size": { "type": "integer", "minimum": 1u64, "maximum": 2147483648u64 },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 3600 },
                "sha256": { "type": "string" }
            },
            "required": ["url", "output"]
        });

        let client = reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            schema,
            client,
        }
    }

    fn parse_args(&self, args: &Value) -> Result<DownloadArgs> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Tool("missing 'url' argument".to_owned()))?
            .to_owned();
        let output = args
            .get("output")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Tool("missing 'output' argument".to_owned()))?
            .to_owned();

        let resume = args.get("resume").and_then(Value::as_bool).unwrap_or(true);
        let chunk_size = args
            .get("chunk_size")
            .and_then(Value::as_u64)
            .unwrap_or(64 * 1024)
            .clamp(1024, 4 * 1024 * 1024) as usize;
        let max_size = args
            .get("max_size")
            .and_then(Value::as_u64)
            .unwrap_or(200 * 1024 * 1024)
            .clamp(1, MAX_DOWNLOAD_BYTES_HARD);
        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 3600);
        let sha256 = args
            .get("sha256")
            .and_then(Value::as_str)
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty());

        let parsed = reqwest::Url::parse(&url)
            .map_err(|err| Error::Tool(format!("invalid url '{url}': {err}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::Tool(
                "download url must use http or https".to_owned(),
            ));
        }

        Ok(DownloadArgs {
            url,
            output,
            resume,
            chunk_size,
            max_size,
            timeout_seconds,
            sha256,
        })
    }

    fn canonicalize(path: &Path) -> Result<PathBuf> {
        fs::canonicalize(path).map_err(|err| {
            Error::Tool(format!(
                "failed to resolve path '{}': {err}",
                path.display()
            ))
        })
    }

    fn resolve_output_path(
        &self,
        context: &ToolExecutionContext,
        raw_output: &str,
    ) -> Result<PathBuf> {
        let workspace = Self::canonicalize(&context.working_directory)?;
        let output_candidate = PathBuf::from(raw_output);
        let requested = if output_candidate.is_absolute() {
            output_candidate
        } else {
            workspace.join(output_candidate)
        };

        let parent = requested.parent().ok_or_else(|| {
            Error::Tool(format!(
                "output path '{}' has no parent directory",
                requested.display()
            ))
        })?;
        if !parent.exists() {
            return Err(Error::Tool(format!(
                "output parent directory '{}' does not exist",
                parent.display()
            )));
        }
        let parent_resolved = Self::canonicalize(parent)?;
        if !parent_resolved.starts_with(&workspace) {
            return Err(Error::Tool(format!(
                "output path '{}' is outside tool working directory '{}'; use a path within the workspace",
                requested.display(),
                workspace.display()
            )));
        }

        Ok(requested)
    }

    fn file_sha256(path: &Path) -> Result<String> {
        let mut file = fs::File::open(path)
            .map_err(|err| Error::Tool(format!("failed to open '{}': {err}", path.display())))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|err| {
                Error::Tool(format!(
                    "failed to read '{}' for sha256: {err}",
                    path.display()
                ))
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(format!("{:x}", hasher.finalize()))
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
                _ = token.cancelled() => Err(Error::Timeout("download cancelled".to_owned())),
                result = timeout(Duration::from_secs(timeout_seconds), operation) => {
                    match result {
                        Ok(inner) => inner,
                        Err(_) => Err(Error::Timeout(format!("download timed out after {timeout_seconds} seconds"))),
                    }
                }
            }
        } else {
            match timeout(Duration::from_secs(timeout_seconds), operation).await {
                Ok(inner) => inner,
                Err(_) => Err(Error::Timeout(format!(
                    "download timed out after {timeout_seconds} seconds"
                ))),
            }
        }
    }

    async fn execute_download(
        &self,
        args: &DownloadArgs,
        context: &ToolExecutionContext,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<Value> {
        let output_path = self.resolve_output_path(context, &args.output)?;
        let mut resume_from = 0u64;
        if args.resume && output_path.exists() {
            let metadata = fs::metadata(&output_path).map_err(|err| {
                Error::Tool(format!(
                    "failed to read metadata for '{}': {err}",
                    output_path.display()
                ))
            })?;
            resume_from = metadata.len();
        }

        let mut request = self.client.get(&args.url);
        if resume_from > 0 {
            request = request.header(RANGE, format!("bytes={resume_from}-"));
        }
        let response = request.send().await.map_err(|err| {
            Error::Tool(format!("download request failed for '{}': {err}", args.url))
        })?;

        let status = response.status();
        if !status.is_success() && status.as_u16() != 206 {
            return Err(Error::Tool(format!(
                "download failed with status {}",
                status
            )));
        }

        let partial = status.as_u16() == 206;
        if resume_from > 0 && !partial {
            return Err(Error::Tool(
                "resume requested but server did not honor Range request (status was not 206)"
                    .to_owned(),
            ));
        }

        let response_len = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let expected_total = response_len.map(|len| len.saturating_add(resume_from));
        if let Some(total) = expected_total {
            if total > args.max_size {
                return Err(Error::Tool(format!(
                    "download size {} exceeds max_size {}",
                    total, args.max_size
                )));
            }
        }

        let mut file = if partial {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&output_path)
                .map_err(|err| {
                    Error::Tool(format!(
                        "failed to open '{}' for append: {err}",
                        output_path.display()
                    ))
                })?
        } else {
            OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&output_path)
                .map_err(|err| {
                    Error::Tool(format!(
                        "failed to open '{}' for write: {err}",
                        output_path.display()
                    ))
                })?
        };

        let mut downloaded_now = 0u64;
        let mut stream = response.bytes_stream();
        let mut progress_tick = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|err| Error::Tool(format!("failed reading download stream: {err}")))?;

            let mut offset = 0usize;
            while offset < chunk.len() {
                let end = usize::min(offset + args.chunk_size, chunk.len());
                file.write_all(&chunk[offset..end]).map_err(|err| {
                    Error::Tool(format!(
                        "failed writing to '{}': {err}",
                        output_path.display()
                    ))
                })?;
                offset = end;
            }

            downloaded_now = downloaded_now.saturating_add(chunk.len() as u64);
            let downloaded_total = resume_from.saturating_add(downloaded_now);
            if downloaded_total > args.max_size {
                return Err(Error::Tool(format!(
                    "download exceeded max_size {} (currently {})",
                    args.max_size, downloaded_total
                )));
            }

            progress_tick = progress_tick.saturating_add(1);
            if progress_tick.is_multiple_of(8) {
                if let Some(tx) = tx.as_ref() {
                    let _ = tx.try_send(Event::ToolOutput {
                        tool: self.config.name.clone(),
                        stdout_chunk: format!(
                            "downloaded {} bytes{}\n",
                            downloaded_total,
                            expected_total
                                .map(|total| format!(" / {total}"))
                                .unwrap_or_default()
                        ),
                        stderr_chunk: String::new(),
                    });
                }
            }
        }

        file.flush().map_err(|err| {
            Error::Tool(format!(
                "failed to flush '{}': {err}",
                output_path.display()
            ))
        })?;
        let final_size = fs::metadata(&output_path)
            .map_err(|err| {
                Error::Tool(format!(
                    "failed to read metadata for '{}': {err}",
                    output_path.display()
                ))
            })?
            .len();

        let computed_sha256 = Self::file_sha256(&output_path)?;
        if let Some(expected_sha256) = &args.sha256 {
            if expected_sha256 != &computed_sha256 {
                return Err(Error::Tool(format!(
                    "sha256 mismatch for '{}': expected {}, got {}",
                    output_path.display(),
                    expected_sha256,
                    computed_sha256
                )));
            }
        }

        Ok(json!({
            "url": args.url,
            "output": output_path.to_string_lossy(),
            "resumed": partial,
            "downloaded_bytes": downloaded_now,
            "total_bytes": final_size,
            "expected_total_bytes": expected_total,
            "status": status.as_u16(),
            "sha256": computed_sha256,
            "sha256_verified": args.sha256.is_some(),
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
                self.execute_download(&parsed, context, tx),
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
impl Tool for DownloadTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Download files with resume, limits, progress, and sha256 verification"
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
                let _ = tx.try_send(Event::Error(format!("download tool failed: {err}")));
            }
        }

        result
    }
}
