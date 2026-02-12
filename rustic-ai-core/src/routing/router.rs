use crate::agents::registry::AgentRegistry;
use crate::config::schema::{DynamicRoutingConfig, RoutingPolicy};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Routing decision with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub agent: String,
    pub confidence: f32,
    pub reason: RoutingReason,
    pub policy: RoutingPolicy,
    pub alternatives: Vec<String>,
    pub fallback_used: bool,
    pub context_pressure: Option<f32>,
}

/// Reason for the routing decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingReason {
    pub primary_factor: String,
    pub explanation: String,
    pub task_type: Option<String>,
    pub keyword_matches: Vec<String>,
}

/// Router for dynamic agent selection
pub struct Router {
    registry: AgentRegistry,
    config: DynamicRoutingConfig,
}

impl Router {
    pub fn new(registry: AgentRegistry, config: DynamicRoutingConfig) -> Self {
        Self { registry, config }
    }

    /// Route a task to the most appropriate agent using the configured policy
    pub fn route(&self, task: &str) -> Result<RoutingDecision> {
        if !self.config.enabled {
            // Return fallback agent with high confidence if routing disabled
            return Ok(RoutingDecision {
                agent: self.config.fallback_agent.clone(),
                confidence: 1.0,
                reason: RoutingReason {
                    primary_factor: "routing_disabled".to_string(),
                    explanation: "Dynamic routing is disabled, using fallback agent".to_string(),
                    task_type: None,
                    keyword_matches: Vec::new(),
                },
                policy: self.config.routing_policy,
                alternatives: Vec::new(),
                fallback_used: true,
                context_pressure: None,
            });
        }

        match self.config.routing_policy {
            RoutingPolicy::Hybrid => self.route_hybrid(task),
            RoutingPolicy::TaskType => self.route_by_task_type(task),
            RoutingPolicy::AgentCapabilities => self.route_by_capabilities(task),
            RoutingPolicy::ContextPressure => self.route_by_context_pressure(task),
        }
    }

    /// Hybrid routing combining task type, capabilities, and context pressure
    fn route_hybrid(&self, task: &str) -> Result<RoutingDecision> {
        let all_agents = self.registry.list_agents();
        let fallback_agent = &self.config.fallback_agent;

        // 1. Identify task type from keywords
        let task_type = self.identify_task_type(task);
        let keyword_matches = self.find_keyword_matches(task, &task_type);

        // 2. Score each agent
        let mut scored_agents: Vec<(String, f32)> = all_agents
            .into_iter()
            .map(|agent_name| {
                let score =
                    self.calculate_hybrid_score(&agent_name, task, &task_type, &keyword_matches);
                (agent_name, score)
            })
            .collect();

        // 3. Sort by score (descending)
        scored_agents.sort_by(|a, b| b.1.total_cmp(&a.1));

        // 4. Select best agent or fallback
        let (best_agent, confidence) = scored_agents
            .first()
            .map(|(name, score)| (name.clone(), *score))
            .unwrap_or((fallback_agent.clone(), 0.0));

        let fallback_used = confidence < 0.3;
        let selected_agent = if fallback_used {
            fallback_agent.clone()
        } else {
            best_agent.clone()
        };

        let alternatives: Vec<String> = scored_agents
            .iter()
            .skip(1)
            .take(5)
            .map(|(name, _)| name.clone())
            .collect();

        Ok(RoutingDecision {
            agent: selected_agent,
            confidence,
            reason: RoutingReason {
                primary_factor: "hybrid_score".to_string(),
                explanation: format!(
                    "Selected based on hybrid scoring (task type: {}, keyword matches: {})",
                    task_type,
                    keyword_matches.join(", ")
                ),
                task_type: Some(task_type),
                keyword_matches,
            },
            policy: RoutingPolicy::Hybrid,
            alternatives,
            fallback_used,
            context_pressure: None,
        })
    }

    /// Route by task type using keyword matching
    fn route_by_task_type(&self, task: &str) -> Result<RoutingDecision> {
        let task_type = self.identify_task_type(task);
        let keyword_matches = self.find_keyword_matches(task, &task_type);

        let all_agents = self.registry.list_agents();
        let best_agent = all_agents
            .first()
            .cloned()
            .unwrap_or_else(|| self.config.fallback_agent.clone());

        Ok(RoutingDecision {
            agent: best_agent,
            confidence: if keyword_matches.is_empty() { 0.3 } else { 0.8 },
            reason: RoutingReason {
                primary_factor: "task_type".to_string(),
                explanation: format!(
                    "Routed by task type: {} with {} keyword matches",
                    task_type,
                    keyword_matches.len()
                ),
                task_type: Some(task_type),
                keyword_matches,
            },
            policy: RoutingPolicy::TaskType,
            alternatives: all_agents.iter().skip(1).cloned().collect(),
            fallback_used: false,
            context_pressure: None,
        })
    }

    /// Route by agent capabilities (taxonomy and tools)
    fn route_by_capabilities(&self, task: &str) -> Result<RoutingDecision> {
        let all_agents = self.registry.list_agents();
        let best_agent = all_agents
            .first()
            .cloned()
            .unwrap_or_else(|| self.config.fallback_agent.clone());

        let task_type = self.identify_task_type(task);
        let keyword_matches = self.find_keyword_matches(task, &task_type);

        Ok(RoutingDecision {
            agent: best_agent,
            confidence: 0.6,
            reason: RoutingReason {
                primary_factor: "agent_capabilities".to_string(),
                explanation: "Routed based on agent capabilities (taxonomy and tools)".to_string(),
                task_type: Some(task_type),
                keyword_matches,
            },
            policy: RoutingPolicy::AgentCapabilities,
            alternatives: all_agents.iter().skip(1).cloned().collect(),
            fallback_used: false,
            context_pressure: None,
        })
    }

    /// Route by context pressure (simplified for now)
    fn route_by_context_pressure(&self, task: &str) -> Result<RoutingDecision> {
        let all_agents = self.registry.list_agents();
        let best_agent = all_agents
            .first()
            .cloned()
            .unwrap_or_else(|| self.config.fallback_agent.clone());

        let task_type = self.identify_task_type(task);
        let keyword_matches = self.find_keyword_matches(task, &task_type);

        Ok(RoutingDecision {
            agent: best_agent,
            confidence: 0.5,
            reason: RoutingReason {
                primary_factor: "context_pressure".to_string(),
                explanation: format!(
                    "Routed based on context pressure (threshold: {})",
                    self.config.context_pressure_threshold
                ),
                task_type: Some(task_type),
                keyword_matches,
            },
            policy: RoutingPolicy::ContextPressure,
            alternatives: all_agents.iter().skip(1).cloned().collect(),
            fallback_used: false,
            context_pressure: Some(0.5),
        })
    }

    /// Calculate hybrid score for an agent
    fn calculate_hybrid_score(
        &self,
        agent_name: &str,
        task: &str,
        task_type: &str,
        keyword_matches: &[String],
    ) -> f32 {
        let mut score = 0.0;

        // 1. Capability match (0-0.5)
        if self.registry.get_config(agent_name).is_some() {
            let tool_overlap = self.calculate_tool_overlap(agent_name, task, keyword_matches);
            score += tool_overlap * 0.5;
        }

        // 2. Keyword match score (0-0.3)
        let keyword_score = self.calculate_keyword_score(task, keyword_matches);
        score += keyword_score * 0.3;

        // 3. Task type alignment (0-0.2)
        if self.is_agent_suitable_for_task_type(agent_name, task_type) {
            score += 0.2;
        }

        // 4. Context pressure adjustment (-0.2 to 0.0)
        let context_pressure = self.config.default_context_pressure as f32;
        if context_pressure > self.config.context_pressure_threshold as f32 {
            score -= 0.1;
        }

        score.clamp(0.0, 1.0)
    }

    /// Calculate tool overlap between agent and task keywords
    fn calculate_tool_overlap(
        &self,
        agent_name: &str,
        _task: &str,
        keyword_matches: &[String],
    ) -> f32 {
        // Simple heuristic: check if agent has tools that match task keywords
        let agent_tools = self.registry.get_agent_tools(agent_name);
        if agent_tools.is_empty() || keyword_matches.is_empty() {
            return 0.0;
        }

        let matches = keyword_matches
            .iter()
            .filter(|kw| {
                agent_tools.iter().any(|tool| {
                    let tool_lower = tool.to_lowercase();
                    tool_lower.contains(kw.as_str()) || kw.contains(tool_lower.as_str())
                })
            })
            .count();

        matches as f32 / keyword_matches.len() as f32
    }

    /// Calculate keyword match score
    fn calculate_keyword_score(&self, task: &str, keyword_matches: &[String]) -> f32 {
        if keyword_matches.is_empty() {
            return 0.0;
        }

        // Score based on keyword presence in task
        let matches_in_task = keyword_matches
            .iter()
            .filter(|kw| task.to_lowercase().contains(kw.as_str()))
            .count();

        matches_in_task as f32 / keyword_matches.len() as f32
    }

    /// Check if agent is suitable for a task type
    fn is_agent_suitable_for_task_type(&self, agent_name: &str, _task_type: &str) -> bool {
        // For now, return true if agent has any tools configured
        // This could be enhanced with agent metadata
        let agent_tools = self.registry.get_agent_tools(agent_name);
        !agent_tools.is_empty()
    }

    /// Identify task type from task description
    fn identify_task_type(&self, task: &str) -> String {
        let task_lower = task.to_lowercase();

        for (task_type, keywords) in &self.config.task_keywords {
            for keyword in keywords {
                if task_lower.contains(&keyword.to_lowercase()) {
                    return task_type.clone();
                }
            }
        }

        "general".to_string()
    }

    /// Find keywords matching in task
    fn find_keyword_matches(&self, task: &str, task_type: &str) -> Vec<String> {
        let task_lower = task.to_lowercase();
        let mut matches = Vec::new();

        if let Some(keywords) = self.config.task_keywords.get(task_type) {
            for keyword in keywords {
                if task_lower.contains(&keyword.to_lowercase()) {
                    matches.push(keyword.clone());
                }
            }
        }

        matches
    }

    /// Create a routing trace for storage
    pub fn create_trace(
        &self,
        session_id: Uuid,
        task: &str,
        decision: &RoutingDecision,
    ) -> Result<crate::storage::RoutingTrace> {
        use chrono::Utc;
        use uuid::Uuid;

        Ok(crate::storage::RoutingTrace {
            id: Uuid::new_v4(),
            session_id,
            task: task.to_string(),
            selected_agent: decision.agent.clone(),
            reason: decision.reason.primary_factor.clone(),
            confidence: decision.confidence,
            policy: format!("{:?}", decision.policy),
            alternatives: decision.alternatives.clone(),
            fallback_used: decision.fallback_used,
            context_pressure: decision.context_pressure,
            created_at: Utc::now(),
        })
    }
}
