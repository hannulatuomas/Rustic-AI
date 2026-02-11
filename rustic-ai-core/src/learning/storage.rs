use uuid::Uuid;

use crate::error::Result;
use crate::storage::StorageBackend;

use super::types::{
    MistakePattern, PatternCategory, PreferenceValue, SuccessPattern, UserFeedback, UserPreference,
};

pub async fn store_feedback(storage: &dyn StorageBackend, feedback: &UserFeedback) -> Result<()> {
    storage.store_user_feedback(feedback).await
}

pub async fn list_feedback(
    storage: &dyn StorageBackend,
    session_id: Uuid,
    limit: usize,
) -> Result<Vec<UserFeedback>> {
    storage.list_user_feedback(session_id, limit).await
}

pub async fn upsert_pattern(storage: &dyn StorageBackend, pattern: &MistakePattern) -> Result<()> {
    storage.upsert_mistake_pattern(pattern).await
}

pub async fn list_patterns(
    storage: &dyn StorageBackend,
    agent_name: &str,
    min_frequency: u32,
    limit: usize,
) -> Result<Vec<MistakePattern>> {
    storage
        .list_mistake_patterns(agent_name, min_frequency, limit)
        .await
}

pub async fn upsert_preference(
    storage: &dyn StorageBackend,
    session_id: Uuid,
    key: &str,
    value: &PreferenceValue,
) -> Result<()> {
    storage.upsert_user_preference(session_id, key, value).await
}

pub async fn get_preference(
    storage: &dyn StorageBackend,
    session_id: Uuid,
    key: &str,
) -> Result<Option<PreferenceValue>> {
    storage.get_user_preference(session_id, key).await
}

pub async fn list_preferences(
    storage: &dyn StorageBackend,
    session_id: Uuid,
) -> Result<Vec<UserPreference>> {
    storage.list_user_preferences(session_id).await
}

pub async fn upsert_success(storage: &dyn StorageBackend, pattern: &SuccessPattern) -> Result<()> {
    storage.upsert_success_pattern(pattern).await
}

pub async fn find_success(
    storage: &dyn StorageBackend,
    agent_name: &str,
    category: Option<PatternCategory>,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<SuccessPattern>> {
    storage
        .find_success_patterns(agent_name, category, query, limit)
        .await
}
