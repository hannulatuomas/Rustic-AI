use crate::error::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ScriptLanguage {
    Python,
    JavaScript,
    TypeScript,
}

#[derive(Debug, Clone)]
pub enum SkillKind {
    Instruction {
        content: String,
    },
    Script {
        path: PathBuf,
        language: ScriptLanguage,
    },
}

#[derive(Debug, Clone)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub timeout_seconds: u64,
    pub kind: SkillKind,
}

#[derive(Debug, Clone)]
pub struct SkillResult {
    pub success: bool,
    pub output: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct SkillExecutionContext {
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
}

#[async_trait]
pub trait Skill: Send + Sync {
    fn spec(&self) -> &SkillSpec;

    async fn execute(&self, input: Value, context: &SkillExecutionContext) -> Result<SkillResult>;
}
