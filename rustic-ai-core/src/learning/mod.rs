mod feedback;
mod patterns;
mod preferences;
pub mod storage;
mod success_patterns;
pub mod types;

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::error::Result;
use crate::events::Event;
use crate::storage::StorageBackend;

pub use types::{
    FeedbackContext, FeedbackType, MistakePattern, MistakeType, PatternCategory, PreferenceValue,
    SuccessPattern, UserFeedback, UserPreference,
};

#[derive(Clone)]
pub struct LearningManager {
    storage: Arc<dyn StorageBackend>,
    enabled: bool,
    warning_frequency_threshold: u32,
}

impl LearningManager {
    pub fn new(storage: Arc<dyn StorageBackend>, enabled: bool) -> Self {
        Self {
            storage,
            enabled,
            warning_frequency_threshold: 3,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub async fn submit_feedback(
        &self,
        session_id: Uuid,
        agent_name: String,
        feedback_type: FeedbackType,
        rating: i8,
        comment: Option<String>,
        context: FeedbackContext,
    ) -> Result<UserFeedback> {
        if !self.enabled {
            return feedback::new_feedback(
                session_id,
                agent_name,
                feedback_type,
                rating,
                comment,
                context,
            );
        }

        let feedback = feedback::new_feedback(
            session_id,
            agent_name,
            feedback_type,
            rating,
            comment,
            context,
        )?;
        self.storage.store_user_feedback(&feedback).await?;
        Ok(feedback)
    }

    pub async fn list_feedback(&self, session_id: Uuid, limit: usize) -> Result<Vec<UserFeedback>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        self.storage.list_user_feedback(session_id, limit).await
    }

    pub async fn record_implicit_event(
        &self,
        session_id: Uuid,
        agent_name: &str,
        event: &Event,
        task_description: Option<String>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        match event {
            Event::ToolCompleted { tool, exit_code } if *exit_code != 0 => {
                let trigger = format!("tool={tool};exit_code={exit_code}");
                let mistake_type = patterns::classify_tool_failure(tool, Some(*exit_code), "");
                self.record_mistake(agent_name, mistake_type, trigger)
                    .await?;
                let context = FeedbackContext {
                    task_description,
                    tools_used: vec![tool.clone()],
                    model_response: None,
                    error_occurred: true,
                    error_message: Some(format!("tool '{tool}' exited with code {exit_code}")),
                };
                let _ = self
                    .submit_feedback(
                        session_id,
                        agent_name.to_owned(),
                        FeedbackType::ImplicitError,
                        -1,
                        None,
                        context,
                    )
                    .await?;
            }
            Event::PermissionDecision { tool, decision, .. }
                if *decision == crate::permissions::AskResolution::Deny =>
            {
                self.record_mistake(
                    agent_name,
                    MistakeType::PermissionDenied,
                    format!("tool={tool};permission=deny"),
                )
                .await?;
                let context = FeedbackContext {
                    task_description,
                    tools_used: vec![tool.clone()],
                    model_response: None,
                    error_occurred: true,
                    error_message: Some(format!("permission denied for tool '{tool}'")),
                };
                let _ = self
                    .submit_feedback(
                        session_id,
                        agent_name.to_owned(),
                        FeedbackType::ImplicitPermissionDenied,
                        -1,
                        None,
                        context,
                    )
                    .await?;
            }
            Event::Error(message) => {
                let mistake_type = patterns::classify_error_message(message);
                self.record_mistake(agent_name, mistake_type, message.clone())
                    .await?;
                let context = FeedbackContext {
                    task_description,
                    tools_used: Vec::new(),
                    model_response: None,
                    error_occurred: true,
                    error_message: Some(message.clone()),
                };
                let _ = self
                    .submit_feedback(
                        session_id,
                        agent_name.to_owned(),
                        FeedbackType::ImplicitError,
                        -1,
                        None,
                        context,
                    )
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }

    pub async fn record_tool_failure(
        &self,
        agent_name: &str,
        tool_name: &str,
        exit_code: Option<i32>,
        output: &str,
    ) -> Result<MistakePattern> {
        let mistake_type = patterns::classify_tool_failure(tool_name, exit_code, output);
        let trigger = format!(
            "tool={tool_name};exit_code={};output={}",
            exit_code.unwrap_or_default(),
            output.chars().take(160).collect::<String>()
        );
        self.record_mistake(agent_name, mistake_type, trigger).await
    }

    pub async fn record_error_message(
        &self,
        agent_name: &str,
        message: &str,
    ) -> Result<MistakePattern> {
        let mistake_type = patterns::classify_error_message(message);
        self.record_mistake(agent_name, mistake_type, message.to_owned())
            .await
    }

    pub async fn record_mistake(
        &self,
        agent_name: &str,
        mistake_type: MistakeType,
        trigger: String,
    ) -> Result<MistakePattern> {
        if !self.enabled {
            return Ok(MistakePattern {
                id: Uuid::new_v4(),
                agent_name: agent_name.to_owned(),
                mistake_type,
                trigger,
                frequency: 1,
                last_seen: Utc::now(),
                suggested_fix: patterns::suggest_fix(mistake_type),
            });
        }

        let existing = self
            .storage
            .list_mistake_patterns(agent_name, 0, 256)
            .await?
            .into_iter()
            .find(|pattern| pattern.mistake_type == mistake_type && pattern.trigger == trigger);

        let pattern = if let Some(mut pattern) = existing {
            pattern.frequency = pattern.frequency.saturating_add(1);
            pattern.last_seen = Utc::now();
            if pattern.suggested_fix.is_none() {
                pattern.suggested_fix = patterns::suggest_fix(mistake_type);
            }
            pattern
        } else {
            MistakePattern {
                id: Uuid::new_v4(),
                agent_name: agent_name.to_owned(),
                mistake_type,
                trigger,
                frequency: 1,
                last_seen: Utc::now(),
                suggested_fix: patterns::suggest_fix(mistake_type),
            }
        };

        self.storage.upsert_mistake_pattern(&pattern).await?;
        Ok(pattern)
    }

    pub async fn get_active_patterns(
        &self,
        agent_name: &str,
        limit: usize,
    ) -> Result<Vec<MistakePattern>> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        self.storage
            .list_mistake_patterns(agent_name, self.warning_frequency_threshold, limit)
            .await
    }

    pub async fn record_choice(
        &self,
        session_id: Uuid,
        key: &str,
        value: PreferenceValue,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        self.storage
            .upsert_user_preference(session_id, key, &value)
            .await
    }

    pub async fn record_rating(&self, session_id: Uuid, task_type: &str, rating: i8) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        feedback::validate_rating(rating)?;
        let key = format!("task_rating.{}", task_type.trim().to_ascii_lowercase());
        self.storage
            .upsert_user_preference(session_id, &key, &PreferenceValue::Int(rating as i64))
            .await
    }

    pub async fn get_preference(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<PreferenceValue>> {
        if !self.enabled {
            return Ok(None);
        }
        self.storage.get_user_preference(session_id, key).await
    }

    pub async fn get_preferred_approach(
        &self,
        session_id: Uuid,
        task_type: &str,
    ) -> Result<Option<String>> {
        if !self.enabled {
            return Ok(None);
        }

        let key = preferences::preferred_approach_key(task_type);
        let value = self.storage.get_user_preference(session_id, &key).await?;
        Ok(match value {
            Some(PreferenceValue::String(value)) => Some(value),
            _ => None,
        })
    }

    pub async fn record_success(
        &self,
        session_id: Uuid,
        agent_name: &str,
        task_description: &str,
        tools_used: &[String],
        model_response: &str,
    ) -> Result<SuccessPattern> {
        let category = success_patterns::infer_category(task_description, tools_used);
        let description = task_description.trim().to_owned();
        let template = success_patterns::extract_template(model_response);
        let name = success_patterns::generate_name(&description);

        if !self.enabled {
            return Ok(SuccessPattern {
                id: Uuid::new_v4(),
                agent_name: agent_name.to_owned(),
                name,
                category,
                description,
                template,
                frequency: 1,
                last_used: Utc::now(),
                success_rate: 1.0,
                created_at: Utc::now(),
            });
        }

        let candidates = self
            .storage
            .find_success_patterns(agent_name, Some(category), Some(&description), 32)
            .await?;

        let matched = candidates
            .into_iter()
            .map(|candidate| {
                let score = success_patterns::similarity(&candidate.description, &description);
                (candidate, score)
            })
            .max_by(|left, right| left.1.total_cmp(&right.1));

        let pattern = if let Some((mut pattern, score)) = matched {
            if score >= 0.45 {
                pattern.frequency = pattern.frequency.saturating_add(1);
                pattern.last_used = Utc::now();
                pattern.success_rate = ((pattern.success_rate * ((pattern.frequency - 1) as f64))
                    + 1.0)
                    / (pattern.frequency as f64);
                pattern.template = template;
                pattern
            } else {
                SuccessPattern {
                    id: Uuid::new_v4(),
                    agent_name: agent_name.to_owned(),
                    name,
                    category,
                    description,
                    template,
                    frequency: 1,
                    last_used: Utc::now(),
                    success_rate: 1.0,
                    created_at: Utc::now(),
                }
            }
        } else {
            SuccessPattern {
                id: Uuid::new_v4(),
                agent_name: agent_name.to_owned(),
                name,
                category,
                description,
                template,
                frequency: 1,
                last_used: Utc::now(),
                success_rate: 1.0,
                created_at: Utc::now(),
            }
        };

        self.storage.upsert_success_pattern(&pattern).await?;

        let quality_key = format!("task_quality.{}", category.as_str());
        self.storage
            .upsert_user_preference(session_id, &quality_key, &PreferenceValue::Float(1.0))
            .await?;

        Ok(pattern)
    }

    pub async fn find_patterns(
        &self,
        agent_name: &str,
        category: Option<PatternCategory>,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SuccessPattern>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        self.storage
            .find_success_patterns(agent_name, category, query, limit)
            .await
    }

    pub async fn get_top_patterns(
        &self,
        agent_name: &str,
        limit: usize,
    ) -> Result<Vec<SuccessPattern>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        self.storage
            .find_success_patterns(agent_name, None, None, limit)
            .await
    }
}
