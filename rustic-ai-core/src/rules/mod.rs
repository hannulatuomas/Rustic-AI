pub mod discovery;
pub mod frontmatter;
pub mod gitignore;
pub mod manual_invocation;
pub mod precedence;
pub mod scope;
pub mod topic_inference;
pub mod topic_tracker;

pub use discovery::discover_rule_and_context_files;
pub use manual_invocation::{extract_manual_invocations, resolve_manual_invocations};
pub use topic_inference::TopicInferenceService;
pub use topic_tracker::TopicTracker;
