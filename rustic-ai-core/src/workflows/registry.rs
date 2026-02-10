use super::types::WorkflowDefinition;
use std::collections::HashMap;

#[derive(Default, Clone)]
pub struct WorkflowRegistry {
    workflows: HashMap<String, WorkflowDefinition>,
}

impl WorkflowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, workflow: WorkflowDefinition) {
        self.workflows.insert(workflow.name.clone(), workflow);
    }

    pub fn get(&self, name: &str) -> Option<&WorkflowDefinition> {
        self.workflows.get(name)
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.workflows.keys().cloned().collect();
        names.sort();
        names
    }
}
