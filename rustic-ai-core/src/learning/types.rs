use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackType {
    Explicit,
    ImplicitError,
    ImplicitSuccess,
    ImplicitPermissionDenied,
}

impl FeedbackType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::ImplicitError => "implicit_error",
            Self::ImplicitSuccess => "implicit_success",
            Self::ImplicitPermissionDenied => "implicit_permission_denied",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackContext {
    pub task_description: Option<String>,
    pub tools_used: Vec<String>,
    pub model_response: Option<String>,
    pub error_occurred: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFeedback {
    pub id: Uuid,
    pub session_id: Uuid,
    pub agent_name: String,
    pub feedback_type: FeedbackType,
    pub rating: i8,
    pub comment: Option<String>,
    pub context: FeedbackContext,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MistakeType {
    PermissionDenied,
    ToolTimeout,
    FileNotFound,
    CompilationError,
    TestFailure,
    WrongApproach,
}

impl MistakeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::ToolTimeout => "tool_timeout",
            Self::FileNotFound => "file_not_found",
            Self::CompilationError => "compilation_error",
            Self::TestFailure => "test_failure",
            Self::WrongApproach => "wrong_approach",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MistakePattern {
    pub id: Uuid,
    pub agent_name: String,
    pub mistake_type: MistakeType,
    pub trigger: String,
    pub frequency: u32,
    pub last_seen: DateTime<Utc>,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum PreferenceValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl PreferenceValue {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Bool(_) => "bool",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreference {
    pub session_id: Uuid,
    pub key: String,
    pub value: PreferenceValue,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatternCategory {
    ErrorFixing,
    Refactoring,
    Debugging,
    FeatureImplementation,
    Testing,
}

impl PatternCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ErrorFixing => "error_fixing",
            Self::Refactoring => "refactoring",
            Self::Debugging => "debugging",
            Self::FeatureImplementation => "feature_implementation",
            Self::Testing => "testing",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessPattern {
    pub id: Uuid,
    pub agent_name: String,
    pub name: String,
    pub category: PatternCategory,
    pub description: String,
    pub template: String,
    pub frequency: u32,
    pub last_used: DateTime<Utc>,
    pub success_rate: f64,
    pub created_at: DateTime<Utc>,
}
