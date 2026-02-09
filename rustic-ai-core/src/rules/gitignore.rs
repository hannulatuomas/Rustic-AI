use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
struct IgnorePattern {
    pattern: String,
    is_negation: bool,
    dir_only: bool,
    anchored: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GitignoreMatcher {
    patterns: Vec<IgnorePattern>,
    root: PathBuf,
}

impl GitignoreMatcher {
    pub fn from_root(root: &Path) -> Result<Self> {
        let mut matcher = Self {
            patterns: Vec::new(),
            root: root.to_path_buf(),
        };

        let gitignore_path = root.join(".gitignore");
        if !gitignore_path.exists() {
            return Ok(matcher);
        }

        let content = std::fs::read_to_string(&gitignore_path).map_err(|err| {
            Error::Config(format!(
                "failed to read .gitignore '{}': {err}",
                gitignore_path.display()
            ))
        })?;

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let is_negation = line.starts_with('!');
            let pattern_without_negation = if is_negation { &line[1..] } else { line };
            if pattern_without_negation.is_empty() {
                continue;
            }

            let anchored = pattern_without_negation.starts_with('/');
            let trimmed_pattern = if anchored {
                &pattern_without_negation[1..]
            } else {
                pattern_without_negation
            };
            let dir_only = trimmed_pattern.ends_with('/');
            let pattern = if dir_only {
                trimmed_pattern.trim_end_matches('/').to_owned()
            } else {
                trimmed_pattern.to_owned()
            };

            if pattern.is_empty() {
                continue;
            }

            matcher.patterns.push(IgnorePattern {
                pattern,
                is_negation,
                dir_only,
                anchored,
            });
        }

        Ok(matcher)
    }

    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if self.patterns.is_empty() {
            return false;
        }

        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        let normalized = normalize_path(relative);
        if normalized.is_empty() {
            return false;
        }

        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.dir_only && !is_dir {
                continue;
            }

            let matched = if pattern.anchored {
                pattern_match(&pattern.pattern, &normalized)
            } else {
                match_any_segment(&pattern.pattern, &normalized)
            };

            if matched {
                ignored = !pattern.is_negation;
            }
        }

        ignored
    }
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn match_any_segment(pattern: &str, normalized_path: &str) -> bool {
    if pattern.contains('/') {
        return pattern_match(pattern, normalized_path);
    }

    if normalized_path
        .split('/')
        .any(|segment| pattern_match(pattern, segment))
    {
        return true;
    }

    let file_name = normalized_path
        .rsplit('/')
        .next()
        .unwrap_or(normalized_path);
    pattern_match(pattern, file_name)
}

fn pattern_match(pattern: &str, text: &str) -> bool {
    let pattern_chars = pattern.chars().collect::<Vec<_>>();
    let text_chars = text.chars().collect::<Vec<_>>();
    wildcard_match(&pattern_chars, &text_chars, 0, 0)
}

fn wildcard_match(pattern: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }

    if pattern[pi] == '*' {
        if pi + 1 < pattern.len() && pattern[pi + 1] == '*' {
            let mut next_pi = pi + 2;
            while next_pi < pattern.len() && pattern[next_pi] == '*' {
                next_pi += 1;
            }
            for idx in ti..=text.len() {
                if wildcard_match(pattern, text, next_pi, idx) {
                    return true;
                }
            }
            return false;
        }

        for idx in ti..=text.len() {
            if idx > ti && text[idx - 1] == '/' {
                break;
            }
            if wildcard_match(pattern, text, pi + 1, idx) {
                return true;
            }
        }
        return false;
    }

    if pattern[pi] == '?' {
        if ti == text.len() || text[ti] == '/' {
            return false;
        }
        return wildcard_match(pattern, text, pi + 1, ti + 1);
    }

    if ti == text.len() || pattern[pi] != text[ti] {
        return false;
    }

    wildcard_match(pattern, text, pi + 1, ti + 1)
}
