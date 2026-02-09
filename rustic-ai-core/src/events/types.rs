#[derive(Debug, Clone)]
pub enum Event {
    Progress(String),
    ModelChunk(String),
    ToolOutput(String),
    SessionUpdated(String),
    Error(String),
}
