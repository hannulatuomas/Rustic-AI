use super::registry::SkillRegistry;
use super::types::{
    ScriptLanguage, Skill, SkillExecutionContext, SkillKind, SkillResult, SkillSpec,
};
use crate::config::schema::{ScriptExecutionMode, SkillsConfig};
use crate::error::{Error, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const OUTPUT_CAPTURE_LIMIT_BYTES: usize = 10 * 1024;

#[derive(Debug, Clone)]
struct InstructionSkill {
    spec: SkillSpec,
}

#[async_trait]
impl Skill for InstructionSkill {
    fn spec(&self) -> &SkillSpec {
        &self.spec
    }

    async fn execute(&self, input: Value, _context: &SkillExecutionContext) -> Result<SkillResult> {
        let content = match &self.spec.kind {
            SkillKind::Instruction { content } => content,
            _ => {
                return Err(Error::Tool(
                    "internal skill kind mismatch for instruction skill".to_owned(),
                ));
            }
        };
        let payload = json!({
            "skill": self.spec.name,
            "description": self.spec.description,
            "input": input,
            "instruction": content
        });
        Ok(SkillResult {
            success: true,
            output: payload.to_string(),
            exit_code: Some(0),
        })
    }
}

#[derive(Debug, Clone)]
struct ScriptSkill {
    spec: SkillSpec,
    execution_mode: ScriptExecutionMode,
}

impl ScriptSkill {
    fn append_bounded(buffer: &mut String, chunk: &str) {
        let remaining = OUTPUT_CAPTURE_LIMIT_BYTES.saturating_sub(buffer.len());
        if remaining == 0 {
            return;
        }
        if chunk.len() <= remaining {
            buffer.push_str(chunk);
            return;
        }
        let mut consumed = 0usize;
        for ch in chunk.chars() {
            let width = ch.len_utf8();
            if consumed + width > remaining {
                break;
            }
            buffer.push(ch);
            consumed += width;
        }
    }

    fn command_for_language(language: &ScriptLanguage) -> &'static str {
        match language {
            ScriptLanguage::Python => "python3",
            ScriptLanguage::JavaScript => "node",
            ScriptLanguage::TypeScript => "tsx",
        }
    }
}

#[async_trait]
impl Skill for ScriptSkill {
    fn spec(&self) -> &SkillSpec {
        &self.spec
    }

    async fn execute(&self, input: Value, context: &SkillExecutionContext) -> Result<SkillResult> {
        if self.execution_mode == ScriptExecutionMode::Disabled {
            return Err(Error::Tool(format!(
                "script skills are disabled by configuration; cannot execute '{}'",
                self.spec.name
            )));
        }
        if self.execution_mode == ScriptExecutionMode::Sandbox {
            return Err(Error::Tool(
                "script sandbox mode requested but not implemented yet".to_owned(),
            ));
        }

        let (path, language) = match &self.spec.kind {
            SkillKind::Script { path, language } => (path, language),
            _ => {
                return Err(Error::Tool(
                    "internal skill kind mismatch for script skill".to_owned(),
                ));
            }
        };

        let mut cmd = Command::new(Self::command_for_language(language));
        cmd.arg(path)
            .current_dir(&context.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if !context.environment.is_empty() {
            cmd.envs(&context.environment);
        }

        let mut child = cmd.spawn().map_err(|err| {
            Error::Tool(format!(
                "failed to spawn script skill '{}' using '{}': {err}",
                self.spec.name,
                path.display()
            ))
        })?;

        if let Some(mut stdin) = child.stdin.take() {
            let payload = serde_json::to_vec(&input)
                .map_err(|err| Error::Tool(format!("failed to serialize skill input: {err}")))?;
            stdin
                .write_all(&payload)
                .await
                .map_err(|err| Error::Tool(format!("failed writing skill input: {err}")))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|err| Error::Tool(format!("failed finalizing skill input: {err}")))?;
            stdin
                .flush()
                .await
                .map_err(|err| Error::Tool(format!("failed flushing skill input: {err}")))?;
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Tool("failed to capture script stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Tool("failed to capture script stderr".to_owned()))?;

        let stdout_task = tokio::spawn(async move {
            let mut captured = String::new();
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let chunk = format!("{line}\n");
                Self::append_bounded(&mut captured, &chunk);
            }
            captured
        });

        let stderr_task = tokio::spawn(async move {
            let mut captured = String::new();
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let chunk = format!("{line}\n");
                Self::append_bounded(&mut captured, &chunk);
            }
            captured
        });

        let wait = timeout(
            std::time::Duration::from_secs(self.spec.timeout_seconds),
            child.wait(),
        )
        .await
        .map_err(|_| {
            let _ = child.start_kill();
            Error::Tool(format!(
                "script skill '{}' timed out after {} seconds",
                self.spec.name, self.spec.timeout_seconds
            ))
        })?
        .map_err(|err| Error::Tool(format!("failed waiting for script skill: {err}")))?;

        let stdout_text = stdout_task
            .await
            .map_err(|err| Error::Tool(format!("stdout join error: {err}")))?;
        let stderr_text = stderr_task
            .await
            .map_err(|err| Error::Tool(format!("stderr join error: {err}")))?;

        let exit_code = wait.code().unwrap_or(-1);
        Ok(SkillResult {
            success: exit_code == 0,
            output: if exit_code == 0 {
                stdout_text
            } else {
                stderr_text
            },
            exit_code: Some(exit_code),
        })
    }
}

pub struct SkillLoader;

impl SkillLoader {
    fn resolve_dir(raw: &str, base: &Path) -> PathBuf {
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }

    fn discover_files(config: &SkillsConfig, base: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for dir in &config.directories {
            let root = Self::resolve_dir(dir, base);
            if !root.exists() || !root.is_dir() {
                continue;
            }

            let mut queue = VecDeque::new();
            queue.push_back((root, 0usize));
            while let Some((current, depth)) = queue.pop_front() {
                let entries = match std::fs::read_dir(&current) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    if file_type.is_dir() {
                        if depth < config.max_discovery_depth {
                            queue.push_back((path, depth + 1));
                        }
                        continue;
                    }
                    if file_type.is_file() {
                        files.push(path);
                    }
                }
            }
        }

        files.sort();
        files
    }

    fn stem_name(path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|value| value.to_str())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn instruction_spec(path: &Path, timeout_seconds: u64) -> Result<Option<SkillSpec>> {
        let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
            return Ok(None);
        };
        if !matches!(ext, "md" | "txt") {
            return Ok(None);
        }
        let Some(name) = Self::stem_name(path) else {
            return Ok(None);
        };
        let content = std::fs::read_to_string(path).map_err(|err| {
            Error::Tool(format!(
                "failed reading instruction skill '{}': {err}",
                path.display()
            ))
        })?;

        Ok(Some(SkillSpec {
            name: format!("skill.{name}"),
            description: format!("Instruction skill loaded from '{}'", path.display()),
            schema: json!({"type": "object", "additionalProperties": true}),
            timeout_seconds,
            kind: SkillKind::Instruction { content },
            taxonomy_membership: Vec::new(),
        }))
    }

    fn script_spec(
        path: &Path,
        timeout_seconds: u64,
        mode: ScriptExecutionMode,
    ) -> Result<Option<SkillSpec>> {
        let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
            return Ok(None);
        };

        let language = match ext {
            "py" => ScriptLanguage::Python,
            "js" => ScriptLanguage::JavaScript,
            "ts" => ScriptLanguage::TypeScript,
            _ => return Ok(None),
        };

        if mode == ScriptExecutionMode::Disabled {
            return Ok(None);
        }

        let canonical_path = std::fs::canonicalize(path).map_err(|err| {
            Error::Tool(format!(
                "failed canonicalizing script skill '{}': {err}",
                path.display()
            ))
        })?;

        let Some(name) = Self::stem_name(path) else {
            return Ok(None);
        };

        Ok(Some(SkillSpec {
            name: format!("skill.{name}"),
            description: format!("Script skill loaded from '{}'", path.display()),
            schema: json!({"type": "object", "additionalProperties": true}),
            timeout_seconds,
            kind: SkillKind::Script {
                path: canonical_path,
                language,
            },
            taxonomy_membership: Vec::new(),
        }))
    }

    pub fn load(config: &SkillsConfig, work_dir: &Path) -> Result<SkillRegistry> {
        let files = Self::discover_files(config, work_dir);
        let mut registry = SkillRegistry::new();

        let mut names = BTreeMap::<String, PathBuf>::new();
        for file in files {
            let spec = match Self::instruction_spec(&file, config.default_timeout_seconds)? {
                Some(v) => Some(v),
                None => Self::script_spec(
                    &file,
                    config.default_timeout_seconds,
                    config.script_execution_mode,
                )?,
            };

            let Some(spec) = spec else {
                continue;
            };

            if let Some(existing) = names.get(&spec.name) {
                return Err(Error::Config(format!(
                    "duplicate skill name '{}' from '{}' and '{}'",
                    spec.name,
                    existing.display(),
                    file.display()
                )));
            }
            names.insert(spec.name.clone(), file.clone());

            let skill: Arc<dyn Skill> = match spec.kind {
                SkillKind::Instruction { .. } => Arc::new(InstructionSkill { spec }),
                SkillKind::Script { .. } => Arc::new(ScriptSkill {
                    spec,
                    execution_mode: config.script_execution_mode,
                }),
            };
            registry.register(skill.spec().name.clone(), skill);
        }

        Ok(registry)
    }
}
