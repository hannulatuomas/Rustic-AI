pub mod cancellation;

use crate::config::schema::Config;
use crate::error::Result;
use crate::events::EventBus;
use crate::providers::create_provider_registry;
use crate::providers::registry::ProviderRegistry;
use crate::tools::registry::ToolRegistry;

pub struct Runtime {
    pub event_bus: EventBus,
    pub providers: ProviderRegistry,
    pub tools: ToolRegistry,
    pub config: Config,
}

impl Runtime {
    pub fn new(config: Config) -> Result<Self> {
        let providers = create_provider_registry(&config)?;

        Ok(Self {
            event_bus: EventBus::default(),
            providers,
            tools: ToolRegistry::default(),
            config,
        })
    }
}
