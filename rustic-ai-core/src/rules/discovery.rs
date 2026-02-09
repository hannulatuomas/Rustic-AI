use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::schema::{DiscoveredRuleConfig, RuleApplicability, RuleConfig, RuleScopeConfig};
use crate::error::{Error, Result};
use crate::rules::frontmatter::parse_json_frontmatter;
use crate::rules::gitignore::GitignoreMatcher;
use crate::rules::precedence::sort_rule_files_by_precedence;

pub fn discover_rule_and_context_files(
    work_dir: &Path,
    rule_config: &RuleConfig,
) -> Result<RuleConfig> {
    let mut global_rules = discover_global_rules(rule_config)?;
    let mut project_rules = discover_project_rules(work_dir, rule_config)?;
    let mut context_files = discover_context_files(work_dir, rule_config)?;

    let mut discovered_rules = Vec::new();
    discovered_rules.append(&mut global_rules);
    discovered_rules.append(&mut project_rules);
    sort_rule_files_by_precedence(&mut discovered_rules, work_dir);

    context_files.sort_by(|left, right| compare_path_proximity(left, right, work_dir));
    dedup_paths(&mut context_files);

    let mut next = rule_config.clone();
    next.discovered_rules = discovered_rules.clone();
    next.global_files = discovered_rules
        .iter()
        .filter(|rule| matches!(rule.scope, RuleScopeConfig::Global))
        .map(|rule| rule.path.clone())
        .collect();
    next.project_files = discovered_rules
        .iter()
        .filter(|rule| matches!(rule.scope, RuleScopeConfig::Project))
        .map(|rule| rule.path.clone())
        .collect();
    next.topic_files = discovered_rules
        .iter()
        .filter(|rule| {
            matches!(rule.scope, RuleScopeConfig::Topic)
                || matches!(rule.applicability, RuleApplicability::ContextSpecific)
        })
        .map(|rule| rule.path.clone())
        .collect();
    next.context_files = context_files
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();

    Ok(next)
}

fn discover_global_rules(rule_config: &RuleConfig) -> Result<Vec<DiscoveredRuleConfig>> {
    let global_root = resolve_global_root(rule_config);
    if !global_root.exists() {
        return Ok(Vec::new());
    }

    let gitignore = GitignoreMatcher::default();
    discover_rules_in_root(
        &global_root,
        &global_root,
        RuleScopeConfig::Global,
        rule_config,
        &gitignore,
    )
}

fn discover_project_rules(
    work_dir: &Path,
    rule_config: &RuleConfig,
) -> Result<Vec<DiscoveredRuleConfig>> {
    let gitignore = if rule_config.use_gitignore {
        GitignoreMatcher::from_root(work_dir)?
    } else {
        GitignoreMatcher::default()
    };

    let mut roots = BTreeSet::new();
    roots.insert(work_dir.to_path_buf());

    let folder_name = if rule_config.project_rules_folder.trim().is_empty() {
        ".agents"
    } else {
        rule_config.project_rules_folder.as_str()
    };
    roots.insert(work_dir.join(folder_name));

    for additional in &rule_config.additional_search_paths {
        if additional.trim().is_empty() {
            continue;
        }

        let path = PathBuf::from(additional);
        let resolved = if path.is_absolute() {
            path
        } else {
            work_dir.join(path)
        };
        roots.insert(resolved);
    }

    let mut all_rules = Vec::new();
    for root in roots {
        if !root.exists() || !root.is_dir() {
            continue;
        }

        let mut rules = discover_rules_in_root(
            &root,
            work_dir,
            RuleScopeConfig::Project,
            rule_config,
            &gitignore,
        )?;
        all_rules.append(&mut rules);
    }

    dedup_rules(&mut all_rules);
    Ok(all_rules)
}

fn discover_context_files(work_dir: &Path, rule_config: &RuleConfig) -> Result<Vec<PathBuf>> {
    let gitignore = if rule_config.use_gitignore {
        GitignoreMatcher::from_root(work_dir)?
    } else {
        GitignoreMatcher::default()
    };

    let mut roots = vec![work_dir.to_path_buf()];
    for additional in &rule_config.additional_search_paths {
        if additional.trim().is_empty() {
            continue;
        }

        let path = PathBuf::from(additional);
        let resolved = if path.is_absolute() {
            path
        } else {
            work_dir.join(path)
        };
        roots.push(resolved);
    }

    let mut files = Vec::new();
    for root in roots {
        if !root.exists() || !root.is_dir() {
            continue;
        }
        scan_context_dir(&root, work_dir, rule_config, 0, &gitignore, &mut files)?;
    }

    dedup_paths(&mut files);
    Ok(files)
}

fn discover_rules_in_root(
    root: &Path,
    work_dir: &Path,
    default_scope: RuleScopeConfig,
    rule_config: &RuleConfig,
    gitignore: &GitignoreMatcher,
) -> Result<Vec<DiscoveredRuleConfig>> {
    let mut files = Vec::new();
    scan_rules_dir(
        root,
        work_dir,
        rule_config,
        0,
        gitignore,
        default_scope,
        &mut files,
    )?;
    Ok(files)
}

fn scan_rules_dir(
    dir: &Path,
    work_dir: &Path,
    rule_config: &RuleConfig,
    depth: usize,
    gitignore: &GitignoreMatcher,
    default_scope: RuleScopeConfig,
    out: &mut Vec<DiscoveredRuleConfig>,
) -> Result<()> {
    if depth > rule_config.max_discovery_depth {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|err| {
        Error::Config(format!(
            "failed to read directory '{}': {err}",
            dir.display()
        ))
    })?;

    for entry in entries {
        let entry =
            entry.map_err(|err| Error::Config(format!("failed to read directory entry: {err}")))?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|err| {
            Error::Config(format!(
                "failed to read metadata for '{}': {err}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            if rule_config.use_gitignore && gitignore.is_ignored(&path, true) {
                continue;
            }
            if !rule_config.recursive_discovery {
                continue;
            }
            scan_rules_dir(
                &path,
                work_dir,
                rule_config,
                depth + 1,
                gitignore,
                default_scope,
                out,
            )?;
            continue;
        }

        if rule_config.use_gitignore && gitignore.is_ignored(&path, false) {
            continue;
        }

        if !is_rule_file(&path, rule_config) {
            continue;
        }

        let content = std::fs::read_to_string(&path).map_err(|err| {
            Error::Config(format!(
                "failed to read rule file '{}': {err}",
                path.display()
            ))
        })?;

        let metadata = parse_json_frontmatter(&content)?;
        let scope = metadata
            .scope_hint
            .as_deref()
            .map(parse_scope)
            .transpose()?
            .unwrap_or_else(|| infer_scope_from_location(&path, work_dir, default_scope));

        out.push(DiscoveredRuleConfig {
            path: path.to_string_lossy().into_owned(),
            scope,
            description: metadata.description,
            globs: metadata.globs,
            always_apply: metadata.always_apply,
            applicability: metadata.applicability,
            topics: metadata.topics,
            priority: metadata.priority,
        });
    }

    Ok(())
}

fn scan_context_dir(
    dir: &Path,
    work_dir: &Path,
    rule_config: &RuleConfig,
    depth: usize,
    gitignore: &GitignoreMatcher,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > rule_config.max_discovery_depth {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|err| {
        Error::Config(format!(
            "failed to read directory '{}': {err}",
            dir.display()
        ))
    })?;

    for entry in entries {
        let entry =
            entry.map_err(|err| Error::Config(format!("failed to read directory entry: {err}")))?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|err| {
            Error::Config(format!(
                "failed to read metadata for '{}': {err}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            if rule_config.use_gitignore && gitignore.is_ignored(&path, true) {
                continue;
            }
            if !rule_config.recursive_discovery {
                continue;
            }
            scan_context_dir(&path, work_dir, rule_config, depth + 1, gitignore, out)?;
            continue;
        }

        if rule_config.use_gitignore && gitignore.is_ignored(&path, false) {
            continue;
        }

        if is_context_file(&path, work_dir, rule_config) {
            out.push(path);
        }
    }

    Ok(())
}

fn is_rule_file(path: &Path, rule_config: &RuleConfig) -> bool {
    if let Some(file_name) = path.file_name().and_then(|value| value.to_str()) {
        if rule_config
            .rule_file_names
            .iter()
            .any(|name| name == file_name)
        {
            return true;
        }
    }

    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    rule_config
        .rule_extensions
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(extension))
}

fn is_context_file(path: &Path, work_dir: &Path, rule_config: &RuleConfig) -> bool {
    if !rule_config.context_file_patterns.is_empty() {
        let relative = path.strip_prefix(work_dir).unwrap_or(path);
        let normalized = normalize_path(relative);
        return rule_config
            .context_file_patterns
            .iter()
            .any(|pattern| simple_glob_match(pattern, &normalized));
    }

    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    rule_config
        .context_extensions
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(extension))
}

fn parse_scope(value: &str) -> Result<RuleScopeConfig> {
    match value.trim().to_ascii_lowercase().as_str() {
        "global" => Ok(RuleScopeConfig::Global),
        "project" => Ok(RuleScopeConfig::Project),
        "topic" | "session" | "topic_session" => Ok(RuleScopeConfig::Topic),
        other => Err(Error::Config(format!(
            "invalid rule scope in frontmatter: '{other}'"
        ))),
    }
}

fn infer_scope_from_location(
    path: &Path,
    work_dir: &Path,
    default_scope: RuleScopeConfig,
) -> RuleScopeConfig {
    let normalized = normalize_path(path.strip_prefix(work_dir).unwrap_or(path));
    if normalized.starts_with(".agents/") || normalized == ".agents" {
        return RuleScopeConfig::Project;
    }
    default_scope
}

fn resolve_global_root(rule_config: &RuleConfig) -> PathBuf {
    let configured = if rule_config.global_rules_path.trim().is_empty() {
        "~/.rustic-ai/rules"
    } else {
        rule_config.global_rules_path.as_str()
    };

    if let Some(suffix) = configured.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(suffix);
        }
    }

    PathBuf::from(configured)
}

fn compare_path_proximity(left: &Path, right: &Path, work_dir: &Path) -> std::cmp::Ordering {
    let left_depth = path_depth(work_dir, left);
    let right_depth = path_depth(work_dir, right);
    left_depth
        .cmp(&right_depth)
        .then_with(|| left.to_string_lossy().cmp(&right.to_string_lossy()))
}

fn path_depth(work_dir: &Path, path: &Path) -> usize {
    path.strip_prefix(work_dir)
        .map(|relative| relative.components().count())
        .unwrap_or(usize::MAX / 2)
}

fn dedup_rules(rules: &mut Vec<DiscoveredRuleConfig>) {
    let mut seen = BTreeSet::new();
    rules.retain(|rule| seen.insert(rule.path.clone()));
}

fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.to_string_lossy().into_owned()));
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.chars().collect::<Vec<_>>();
    let t = text.chars().collect::<Vec<_>>();
    glob_match_impl(&p, &t, 0, 0)
}

fn glob_match_impl(pattern: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }

    if pattern[pi] == '*' {
        if pi + 1 < pattern.len() && pattern[pi + 1] == '*' {
            let mut next = pi + 2;
            while next < pattern.len() && pattern[next] == '*' {
                next += 1;
            }
            for idx in ti..=text.len() {
                if glob_match_impl(pattern, text, next, idx) {
                    return true;
                }
            }
            return false;
        }

        for idx in ti..=text.len() {
            if idx > ti && text[idx - 1] == '/' {
                break;
            }
            if glob_match_impl(pattern, text, pi + 1, idx) {
                return true;
            }
        }
        return false;
    }

    if pattern[pi] == '?' {
        if ti == text.len() || text[ti] == '/' {
            return false;
        }
        return glob_match_impl(pattern, text, pi + 1, ti + 1);
    }

    if ti >= text.len() || pattern[pi] != text[ti] {
        return false;
    }

    glob_match_impl(pattern, text, pi + 1, ti + 1)
}
