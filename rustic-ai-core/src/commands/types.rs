#[derive(Debug, Clone)]
pub enum Command {
    UserInput(String),
    Slash(String),
    RunWorkflow(String),
}
