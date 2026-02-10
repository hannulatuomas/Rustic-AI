use crate::config::schema::{DecisionScope, PermissionConfig, PermissionMode};
use crate::permissions::policy::{AskResolution, PermissionDecision, PermissionPolicy};

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ConfigurablePermissionPolicy {
    config: PermissionConfig,
    tool_specific_modes: HashMap<String, PermissionMode>,
    denied_cache: HashMap<(String, String), u64>, // (tool, args_signature) -> expiry_timestamp
    allowed_cache: HashMap<(String, String), DecisionScope>, // (tool, args_signature) -> scope
}

impl ConfigurablePermissionPolicy {
    pub fn new(config: PermissionConfig, tool_configs: Vec<(String, PermissionMode)>) -> Self {
        let mut tool_specific_modes = HashMap::new();
        for (tool_name, mode) in tool_configs {
            tool_specific_modes.insert(tool_name, mode);
        }

        Self {
            config,
            tool_specific_modes,
            denied_cache: HashMap::new(),
            allowed_cache: HashMap::new(),
        }
    }

    fn get_tool_mode(&self, tool: &str) -> PermissionMode {
        self.tool_specific_modes
            .get(tool)
            .copied()
            .unwrap_or(self.config.default_tool_permission)
    }

    fn args_signature(&self, args: &serde_json::Value) -> String {
        // Create a simple signature of args for caching
        // For now, we just serialize to string; in production, hash it
        format!("{:?}", args)
    }

    fn check_denied_cache(&self, tool: &str, args_sig: &str) -> bool {
        if let Some(&expiry) = self
            .denied_cache
            .get(&(tool.to_string(), args_sig.to_string()))
        {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now < expiry {
                return true; // Still denied
            }
        }
        false
    }

    fn check_allowed_cache(&self, tool: &str, args_sig: &str) -> Option<AskResolution> {
        if let Some(&scope) = self
            .allowed_cache
            .get(&(tool.to_string(), args_sig.to_string()))
        {
            return match scope {
                DecisionScope::Session => Some(AskResolution::AllowInSession),
                DecisionScope::Project | DecisionScope::Global => Some(AskResolution::AllowOnce),
            };
        }
        None
    }
}

impl PermissionPolicy for ConfigurablePermissionPolicy {
    fn check_tool_permission(&self, tool: &str, args: &serde_json::Value) -> PermissionDecision {
        let mode = self.get_tool_mode(tool);

        match mode {
            PermissionMode::Allow => PermissionDecision::Allow,
            PermissionMode::Deny => PermissionDecision::Deny,
            PermissionMode::Ask => {
                let args_sig = self.args_signature(args);

                // Check denied cache first
                if self.check_denied_cache(tool, &args_sig) {
                    return PermissionDecision::Deny;
                }

                // Check allowed cache
                if let Some(resolution) = self.check_allowed_cache(tool, &args_sig) {
                    return match resolution {
                        AskResolution::AllowOnce => PermissionDecision::Allow,
                        AskResolution::AllowInSession => PermissionDecision::Allow,
                        AskResolution::Deny => PermissionDecision::Deny,
                    };
                }

                // Need to ask user
                PermissionDecision::Ask
            }
        }
    }

    fn record_permission(&mut self, tool: &str, args: &serde_json::Value, decision: AskResolution) {
        let args_sig = self.args_signature(args);

        match decision {
            AskResolution::AllowOnce | AskResolution::AllowInSession => {
                let scope = match decision {
                    AskResolution::AllowInSession => DecisionScope::Session,
                    _ => self.config.ask_decisions_persist_scope,
                };
                self.allowed_cache
                    .insert((tool.to_string(), args_sig), scope);
            }
            AskResolution::Deny => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let expiry = now + self.config.remember_denied_duration_secs;
                self.denied_cache
                    .insert((tool.to_string(), args_sig), expiry);
            }
        }
    }
}
