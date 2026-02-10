use super::types::Skill;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct SkillRegistry {
    skills: HashMap<String, Arc<dyn Skill>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: String, skill: Arc<dyn Skill>) {
        self.skills.insert(name, skill);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        self.skills.get(name).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.skills.keys().cloned().collect();
        names.sort();
        names
    }
}
