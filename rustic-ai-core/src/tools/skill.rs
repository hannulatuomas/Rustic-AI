use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::skills::{SkillExecutionContext, SkillRegistry};
use crate::tools::{Tool, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct SkillTool {
    config: ToolConfig,
    schema: Value,
    skills: Arc<SkillRegistry>,
}

impl SkillTool {
    pub fn new(config: ToolConfig, skills: Arc<SkillRegistry>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "skill": {"type": "string", "description": "Registered skill name"},
                "input": {"type": "object", "description": "Skill input payload"}
            },
            "required": ["skill"]
        });
        Self {
            config,
            schema,
            skills,
        }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Execute a registered skill by name"
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
        let skill_name = args
            .get("skill")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'skill' argument".to_owned()))?;
        let input = args
            .get("input")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        let skill = self
            .skills
            .get(skill_name)
            .ok_or_else(|| Error::NotFound(format!("skill '{}' not found", skill_name)))?;

        let _ = tx.try_send(Event::ToolStarted {
            tool: self.config.name.clone(),
            args: args.clone(),
        });

        let result = skill
            .execute(
                input,
                &SkillExecutionContext {
                    working_directory: context.working_directory.clone(),
                    environment: Default::default(),
                },
            )
            .await?;

        let _ = tx.try_send(Event::ToolCompleted {
            tool: self.config.name.clone(),
            exit_code: result.exit_code.unwrap_or_default(),
        });

        Ok(ToolResult {
            success: result.success,
            exit_code: result.exit_code,
            output: result.output,
        })
    }
}
