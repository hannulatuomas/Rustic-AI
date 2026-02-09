use std::path::Path;

use crate::config::schema::{DiscoveredRuleConfig, RuleScopeConfig};

pub fn sort_rule_files_by_precedence(rules: &mut [DiscoveredRuleConfig], work_dir: &Path) {
    rules.sort_by(|left, right| {
        right.always_apply.cmp(&left.always_apply).then_with(|| {
            scope_rank(&left.scope)
                .cmp(&scope_rank(&right.scope))
                .then_with(|| {
                    proximity_depth(work_dir, &left.path)
                        .cmp(&proximity_depth(work_dir, &right.path))
                })
                .then_with(|| right.priority.unwrap_or(0).cmp(&left.priority.unwrap_or(0)))
                .then_with(|| left.path.cmp(&right.path))
        })
    });
}

fn scope_rank(scope: &RuleScopeConfig) -> usize {
    match scope {
        RuleScopeConfig::Global => 0,
        RuleScopeConfig::Project => 1,
        RuleScopeConfig::Topic => 2,
    }
}

fn proximity_depth(work_dir: &Path, raw_path: &str) -> usize {
    let path = std::path::Path::new(raw_path);
    path.strip_prefix(work_dir)
        .map(|relative| relative.components().count())
        .unwrap_or(usize::MAX / 2)
}
