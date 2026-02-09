#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub source_path: String,
    pub runtime: SkillRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillRuntime {
    Instruction,
    Python,
    JavaScript,
    TypeScript,
}
