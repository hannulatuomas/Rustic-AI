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
}
