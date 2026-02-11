use std::collections::HashMap;

use crate::config::schema::{AgentConfig, BasketConfig, TaxonomyMembershipConfig, ToolConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaxonomyItemKind {
    Agent,
    Tool,
    Skill,
}

#[derive(Debug, Clone)]
pub struct TaxonomyItem {
    pub id: String,
    pub display_name: String,
    pub kind: TaxonomyItemKind,
}

#[derive(Debug, Clone)]
pub struct TaxonomyRegistry {
    baskets: HashMap<String, BasketConfig>,
    memberships: HashMap<(TaxonomyItemKind, String), Vec<TaxonomyMembershipConfig>>,
    items: HashMap<(TaxonomyItemKind, String), TaxonomyItem>,
}

impl TaxonomyRegistry {
    pub fn new(baskets: Vec<BasketConfig>) -> Self {
        let baskets = baskets
            .into_iter()
            .map(|basket| (basket.name.clone(), basket))
            .collect::<HashMap<_, _>>();

        Self {
            baskets,
            memberships: HashMap::new(),
            items: HashMap::new(),
        }
    }

    pub fn register_agent(&mut self, agent: &AgentConfig) {
        self.items.insert(
            (TaxonomyItemKind::Agent, agent.name.clone()),
            TaxonomyItem {
                id: agent.name.clone(),
                display_name: agent.name.clone(),
                kind: TaxonomyItemKind::Agent,
            },
        );
        self.memberships.insert(
            (TaxonomyItemKind::Agent, agent.name.clone()),
            agent.taxonomy_membership.clone(),
        );
    }

    pub fn register_tool(&mut self, tool: &ToolConfig) {
        self.items.insert(
            (TaxonomyItemKind::Tool, tool.name.clone()),
            TaxonomyItem {
                id: tool.name.clone(),
                display_name: tool.name.clone(),
                kind: TaxonomyItemKind::Tool,
            },
        );
        self.memberships.insert(
            (TaxonomyItemKind::Tool, tool.name.clone()),
            tool.taxonomy_membership.clone(),
        );
    }

    pub fn register_skill(&mut self, skill_name: &str, membership: Vec<TaxonomyMembershipConfig>) {
        self.items.insert(
            (TaxonomyItemKind::Skill, skill_name.to_owned()),
            TaxonomyItem {
                id: skill_name.to_owned(),
                display_name: skill_name.to_owned(),
                kind: TaxonomyItemKind::Skill,
            },
        );
        self.memberships
            .insert((TaxonomyItemKind::Skill, skill_name.to_owned()), membership);
    }

    pub fn list_baskets(&self) -> Vec<String> {
        let mut names = self.baskets.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn list_sub_baskets(&self, basket: &str) -> Option<Vec<String>> {
        self.baskets.get(basket).map(|config| {
            let mut values = config.sub_baskets.clone();
            values.sort();
            values
        })
    }

    pub fn find_by_basket(&self, basket: &str) -> Vec<TaxonomyItem> {
        self.items
            .iter()
            .filter_map(|(key, item)| {
                self.memberships
                    .get(key)
                    .map(|values| values.iter().any(|value| value.basket == basket))
                    .unwrap_or(false)
                    .then_some(item.clone())
            })
            .collect()
    }

    pub fn find_by_sub_basket(&self, basket: &str, sub_basket: &str) -> Vec<TaxonomyItem> {
        self.items
            .iter()
            .filter_map(|(key, item)| {
                self.memberships
                    .get(key)
                    .map(|values| {
                        values.iter().any(|value| {
                            value.basket == basket
                                && value
                                    .sub_basket
                                    .as_deref()
                                    .map(|sub| sub == sub_basket)
                                    .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
                    .then_some(item.clone())
            })
            .collect()
    }

    pub fn search(&self, query: &str) -> Vec<TaxonomyItem> {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return Vec::new();
        }

        self.items
            .values()
            .filter(|item| item.display_name.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect()
    }
}
