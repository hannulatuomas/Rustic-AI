#[derive(Debug, Clone)]
pub struct ProjectProfile {
    pub name: String,
    pub root_path: String,
    pub tech_stack: Vec<String>,
    pub goals: Vec<String>,
    pub preferences: Vec<String>,
    pub style_guidelines: Vec<String>,
}
