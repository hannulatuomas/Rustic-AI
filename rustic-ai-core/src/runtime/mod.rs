use crate::agents::AgentCoordinator;
use crate::conversation::session_manager::SessionManager;
use crate::error::Result;
use crate::events::EventBus;
use crate::permissions::{ConfigurablePermissionPolicy, PermissionPolicy};
use crate::providers::create_provider_registry;
use crate::providers::registry::ProviderRegistry;
use crate::skills::{SkillLoader, SkillRegistry};
use crate::tools::{ToolExecutionContext, ToolManager, ToolManagerInit};
use crate::workflows::{WorkflowLoader, WorkflowRegistry};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Runtime {
    pub event_bus: EventBus,
    pub providers: ProviderRegistry,
    pub agents: AgentCoordinator,
    pub tools: Arc<ToolManager>,
    pub skills: Arc<SkillRegistry>,
    pub workflows: Arc<WorkflowRegistry>,
    pub config: crate::config::Config,
}

impl Runtime {
    pub fn new(
        config: crate::config::Config,
        session_manager: Arc<SessionManager>,
    ) -> Result<Self> {
        let work_dir = std::env::current_dir()
            .map_err(|err| crate::Error::Config(format!("failed to resolve current dir: {err}")))?;
        let providers = create_provider_registry(&config, &work_dir)?;

        let skills = if config.features.skills_enabled {
            Arc::new(SkillLoader::load(&config.skills, &work_dir)?)
        } else {
            Arc::new(SkillRegistry::new())
        };
        let workflows = if config.features.workflows_enabled {
            Arc::new(WorkflowLoader::load(&config.workflows, &work_dir)?)
        } else {
            Arc::new(WorkflowRegistry::new())
        };

        // Create permission policy
        let tool_specific_modes: Vec<(String, crate::config::schema::PermissionMode)> = config
            .tools
            .iter()
            .map(|tc| (tc.name.clone(), tc.permission_mode))
            .collect();

        let mut agent_tool_allowlist: HashMap<String, HashSet<String>> = HashMap::new();
        for agent in &config.agents {
            agent_tool_allowlist.insert(agent.name.clone(), agent.tools.iter().cloned().collect());
        }

        let permission_policy: Box<dyn PermissionPolicy + Send + Sync> =
            Box::new(ConfigurablePermissionPolicy::new(
                config.permissions.clone(),
                tool_specific_modes,
                work_dir.clone(),
                agent_tool_allowlist,
            ));

        // Create tool manager
        let tools = Arc::new(ToolManager::new(ToolManagerInit {
            permission_policy,
            permission_config: Arc::new(config.permissions.clone()),
            mcp_enabled: config.features.mcp_enabled,
            mcp_config: Arc::new(config.mcp.clone()),
            skills_enabled: config.features.skills_enabled,
            skills: skills.clone(),
            plugins_enabled: config.features.plugins_enabled,
            plugin_config: Arc::new(config.plugins.clone()),
            tool_configs: config.tools.clone(),
            execution_context: ToolExecutionContext {
                working_directory: work_dir,
            },
        }));

        // Create agent coordinator
        let agents = AgentCoordinator::new(
            config.agents.clone(),
            &providers,
            tools.clone(),
            session_manager.clone(),
        )?;

        Ok(Self {
            event_bus: EventBus::default(),
            providers,
            agents,
            tools,
            skills,
            workflows,
            config,
        })
    }
}
