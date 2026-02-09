use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::types::Tool;

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register(&mut self, name: String, tool: Arc<dyn Tool>) {
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
