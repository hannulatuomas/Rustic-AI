pub mod behavior;
pub mod context;
pub mod coordinator;
pub mod memory;
pub mod registry;
pub mod state;
mod todo_extractor;

pub use behavior::Agent;
pub use coordinator::AgentCoordinator;
pub use registry::{AgentRegistry, AgentSuggestion};
