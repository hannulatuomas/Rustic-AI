use std::collections::BTreeSet;

use super::types::PatternCategory;

pub fn infer_category(task_description: &str, tools_used: &[String]) -> PatternCategory {
    let task = task_description.to_ascii_lowercase();
    let has_tool = |needle: &str| {
        tools_used
            .iter()
            .any(|tool| tool.to_ascii_lowercase().contains(needle))
    };

    if task.contains("test") || has_tool("test") {
        return PatternCategory::Testing;
    }
    if task.contains("debug") || task.contains("error") {
        return PatternCategory::Debugging;
    }
    if task.contains("refactor") || has_tool("git") {
        return PatternCategory::Refactoring;
    }
    if task.contains("fix") || task.contains("bug") {
        return PatternCategory::ErrorFixing;
    }

    PatternCategory::FeatureImplementation
}

pub fn generate_name(task_description: &str) -> String {
    let words = task_description
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    if words.is_empty() {
        "task_completion_pattern".to_owned()
    } else {
        words
    }
}

pub fn extract_template(model_response: &str) -> String {
    let lines = model_response
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(8)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        model_response.chars().take(400).collect()
    } else {
        lines.join("\n")
    }
}

pub fn similarity(left: &str, right: &str) -> f64 {
    let left_tokens = tokenize(left);
    let right_tokens = tokenize(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }

    let intersection = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn tokenize(input: &str) -> BTreeSet<String> {
    input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}
