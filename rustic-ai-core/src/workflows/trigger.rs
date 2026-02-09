#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerType {
    Manual,
    Schedule,
    Webhook,
    Event,
}
