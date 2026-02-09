use std::collections::HashMap;
use std::sync::Arc;

use crate::providers::types::ModelProvider;

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
}

impl ProviderRegistry {
    pub fn register(&mut self, name: String, provider: Arc<dyn ModelProvider>) {
        self.providers.insert(name, provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ModelProvider>> {
        self.providers.get(name).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}
