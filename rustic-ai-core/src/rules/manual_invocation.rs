use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::config::schema::DiscoveredRuleConfig;

pub fn extract_manual_invocations(text: &str) -> Vec<String> {
    static MATCHER: OnceLock<Option<Regex>> = OnceLock::new();
    let Some(matcher) = MATCHER
        .get_or_init(|| Regex::new(r"@([A-Za-z0-9_./\\-]+)").ok())
        .as_ref()
    else {
        return Vec::new();
    };
    matcher
        .captures_iter(text)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
        .collect()
}

pub fn resolve_manual_invocations(
    invocations: &[String],
    discovered_rules: &[DiscoveredRuleConfig],
) -> Vec<String> {
    let mut resolved = Vec::new();

    for invocation in invocations {
        if let Some(rule_path) = resolve_single_invocation(invocation, discovered_rules) {
            if !resolved.iter().any(|path| path == &rule_path) {
                resolved.push(rule_path);
            }
        }
    }

    resolved
}

fn resolve_single_invocation(
    invocation: &str,
    discovered_rules: &[DiscoveredRuleConfig],
) -> Option<String> {
    for rule in discovered_rules {
        if rule.path.ends_with(invocation) {
            return Some(rule.path.clone());
        }

        let file_name = Path::new(&rule.path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let stem = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(file_name);

        if file_name == invocation || stem == invocation {
            return Some(rule.path.clone());
        }
    }

    None
}
