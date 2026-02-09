pub mod cancellation;

use crate::config::schema::Config;
use crate::events::EventBus;
use crate::providers::registry::ProviderRegistry;
use crate::tools::registry::ToolRegistry;

pub struct Runtime {
    pub event_bus: EventBus,
    pub providers: ProviderRegistry,
    pub tools: ToolRegistry,
    pub config: Config,
}

impl Runtime {
    pub fn new(config: Config) -> Self {
        Self {
            event_bus: EventBus::default(),
            providers: ProviderRegistry::default(),
            tools: ToolRegistry::default(),
            config,
        }
    }
}
