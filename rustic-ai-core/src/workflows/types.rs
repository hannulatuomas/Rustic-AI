#[derive(Debug, Clone)]
pub struct Workflow {
    pub name: String,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone)]
pub enum WorkflowStep {
    Agent(String),
    Tool(String),
    Parallel(Vec<WorkflowStep>),
}
