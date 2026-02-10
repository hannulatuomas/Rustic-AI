use crate::agents::AgentCoordinator;
use crate::conversation::session_manager::SessionManager;
use crate::error::Result;
use crate::events::EventBus;
use crate::permissions::{ConfigurablePermissionPolicy, PermissionPolicy};
use crate::providers::create_provider_registry;
use crate::providers::registry::ProviderRegistry;
use crate::ToolManager;
use std::sync::Arc;

pub struct Runtime {
    pub event_bus: EventBus,
    pub providers: ProviderRegistry,
    pub agents: AgentCoordinator,
    pub tools: Arc<ToolManager>,
    pub config: crate::config::Config,
}

impl Runtime {
    pub fn new(
        config: crate::config::Config,
        session_manager: Arc<SessionManager>,
    ) -> Result<Self> {
        let providers = create_provider_registry(&config)?;

        // Create permission policy
        let tool_specific_modes: Vec<(String, crate::config::schema::PermissionMode)> = config
            .tools
            .iter()
            .map(|tc| (tc.name.clone(), tc.permission_mode))
            .collect();

        let permission_policy: Box<dyn PermissionPolicy + Send + Sync> = Box::new(
            ConfigurablePermissionPolicy::new(config.permissions.clone(), tool_specific_modes),
        );

        // Create tool manager
        let tools = Arc::new(ToolManager::new(permission_policy, config.tools.clone()));

        // Create agent coordinator
        let agents = AgentCoordinator::new(
            config.agents.clone(),
            &providers,
            tools.clone(),
            session_manager,
        )?;

        Ok(Self {
            event_bus: EventBus::default(),
            providers,
            agents,
            tools,
            config,
        })
    }
}
