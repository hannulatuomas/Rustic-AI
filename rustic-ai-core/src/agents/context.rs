use std::collections::HashSet;

use crate::providers::types::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageImportance {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ContextOptimizationProfile {
    #[default]
    Balanced,
    Debug,
    Planning,
}

#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub key: String,
    pub content: String,
    pub source_message_count: usize,
    pub generated_with_provider: bool,
}

pub struct MessageScorer;

impl MessageScorer {
    pub fn score(
        message: &ChatMessage,
        task_keywords: &[String],
        profile: ContextOptimizationProfile,
    ) -> MessageImportance {
        let role = message.role.as_str();
        let content_lower = message.content.to_lowercase();

        if role == "system" {
            return MessageImportance::Critical;
        }

        if role == "tool"
            || content_lower.contains("error")
            || content_lower.contains("failed")
            || content_lower.contains("permission")
        {
            return MessageImportance::High;
        }

        if matches!(profile, ContextOptimizationProfile::Debug)
            && (role == "tool"
                || content_lower.contains("trace")
                || content_lower.contains("panic"))
        {
            return MessageImportance::Critical;
        }

        if matches!(profile, ContextOptimizationProfile::Planning)
            && (content_lower.contains("plan") || content_lower.contains("design"))
        {
            return MessageImportance::High;
        }

        if task_keywords
            .iter()
            .any(|keyword| content_lower.contains(keyword))
        {
            return MessageImportance::High;
        }

        if role == "user" {
            return MessageImportance::Medium;
        }

        if role == "assistant" {
            return MessageImportance::Medium;
        }

        MessageImportance::Low
    }

    pub fn profile_from_messages(messages: &[ChatMessage]) -> ContextOptimizationProfile {
        let Some(last_user) = messages.iter().rev().find(|message| message.role == "user") else {
            return ContextOptimizationProfile::Balanced;
        };
        let text = last_user.content.to_lowercase();
        if text.contains("debug")
            || text.contains("fix")
            || text.contains("error")
            || text.contains("panic")
        {
            return ContextOptimizationProfile::Debug;
        }
        if text.contains("plan") || text.contains("design") || text.contains("roadmap") {
            return ContextOptimizationProfile::Planning;
        }
        ContextOptimizationProfile::Balanced
    }
}

pub struct ContextDeduplicator;

impl ContextDeduplicator {
    pub fn deduplicate(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let mut seen = HashSet::new();
        let mut kept = Vec::new();

        for message in messages.into_iter().rev() {
            let key = format!("{}::{}", message.role, message.content);
            if seen.insert(key) {
                kept.push(message);
            }
        }

        kept.reverse();
        kept
    }
}
