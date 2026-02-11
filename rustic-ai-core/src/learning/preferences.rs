pub fn preferred_approach_key(task_type: &str) -> String {
    format!(
        "preferred_approach.{}",
        task_type.trim().to_ascii_lowercase()
    )
}
