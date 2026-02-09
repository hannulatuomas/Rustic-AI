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
}
