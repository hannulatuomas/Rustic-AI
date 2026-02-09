use serde::Deserialize;

use crate::config::schema::RuleApplicability;
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct ParsedRuleMetadata {
    pub description: Option<String>,
    pub globs: Vec<String>,
    pub always_apply: bool,
    pub applicability: RuleApplicability,
    pub topics: Vec<String>,
    pub scope_hint: Option<String>,
    pub priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct RuleFrontmatter {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    globs: Vec<String>,
    #[serde(default, alias = "alwaysApply")]
    always_apply: bool,
    #[serde(default)]
    applicability: RuleApplicability,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    priority: Option<i32>,
}

pub fn parse_json_frontmatter(content: &str) -> Result<ParsedRuleMetadata> {
    let (json_block, _) = split_frontmatter_and_body(content)?;
    let parsed: RuleFrontmatter = serde_json::from_str(&json_block)
        .map_err(|err| Error::Config(format!("failed to parse JSON frontmatter: {err}")))?;

    Ok(ParsedRuleMetadata {
        description: parsed.description,
        globs: parsed.globs,
        always_apply: parsed.always_apply,
        applicability: if parsed.always_apply {
            RuleApplicability::General
        } else if parsed.topics.is_empty() {
            parsed.applicability
        } else {
            RuleApplicability::ContextSpecific
        },
        topics: parsed.topics,
        scope_hint: parsed.scope,
        priority: parsed.priority,
    })
}

fn split_frontmatter_and_body(content: &str) -> Result<(String, String)> {
    let mut lines = content.lines();
    let first = lines.next().unwrap_or_default().trim();
    if first != "---" {
        return Ok(("{}".to_owned(), content.to_owned()));
    }

    let mut json_lines = Vec::new();
    let mut found_end = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            found_end = true;
            break;
        }
        json_lines.push(line);
    }

    if !found_end {
        return Err(Error::Config(
            "frontmatter starts with '---' but does not contain a closing '---'".to_owned(),
        ));
    }

    let body = lines.collect::<Vec<_>>().join("\n");
    let json_block = json_lines.join("\n");

    if json_block.trim().is_empty() {
        return Ok(("{}".to_owned(), body));
    }

    Ok((json_block, body))
}
