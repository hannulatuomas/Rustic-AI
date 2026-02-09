#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    Global,
    Project,
    Topic,
}
