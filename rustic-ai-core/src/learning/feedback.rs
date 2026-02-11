use chrono::Utc;
use uuid::Uuid;

use crate::error::{Error, Result};

use super::types::{FeedbackContext, FeedbackType, UserFeedback};

pub fn validate_rating(rating: i8) -> Result<()> {
    if (-1..=1).contains(&rating) {
        Ok(())
    } else {
        Err(Error::Validation(
            "feedback rating must be between -1 and 1".to_owned(),
        ))
    }
}

pub fn new_feedback(
    session_id: Uuid,
    agent_name: String,
    feedback_type: FeedbackType,
    rating: i8,
    comment: Option<String>,
    context: FeedbackContext,
) -> Result<UserFeedback> {
    validate_rating(rating)?;
    Ok(UserFeedback {
        id: Uuid::new_v4(),
        session_id,
        agent_name,
        feedback_type,
        rating,
        comment,
        context,
        created_at: Utc::now(),
    })
}
