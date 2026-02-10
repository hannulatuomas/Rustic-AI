use super::registry::WorkflowRegistry;
use super::types::{WorkflowDefinition, WorkflowStepKind};
use crate::config::schema::WorkflowsConfig;
use crate::error::{Error, Result};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

pub struct WorkflowLoader;

impl WorkflowLoader {
    fn resolve_dir(raw: &str, base: &Path) -> PathBuf {
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }

    fn discover_files(config: &WorkflowsConfig, base: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for dir in &config.directories {
            let root = Self::resolve_dir(dir, base);
            if !root.exists() || !root.is_dir() {
                continue;
            }

            let mut queue = VecDeque::new();
            queue.push_back((root, 0usize));
            while let Some((current, depth)) = queue.pop_front() {
                let entries = match std::fs::read_dir(&current) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if depth < config.max_discovery_depth {
                            queue.push_back((path, depth + 1));
                        }
                        continue;
                    }
                    if !path.is_file() {
                        continue;
                    }

                    let ext = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    if matches!(ext, "json" | "yaml" | "yml") {
                        files.push(path);
                    }
                }
            }
        }
        files
    }

    fn parse_file(path: &Path) -> Result<WorkflowDefinition> {
        let raw = std::fs::read_to_string(path).map_err(|err| {
            Error::Config(format!(
                "failed reading workflow file '{}': {err}",
                path.display()
            ))
        })?;

        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();

        if matches!(ext, "yaml" | "yml") {
            serde_yaml::from_str(&raw).map_err(|err| {
                Error::Config(format!(
                    "failed parsing workflow yaml '{}': {err}",
                    path.display()
                ))
            })
        } else {
            serde_json::from_str(&raw).map_err(|err| {
                Error::Config(format!(
                    "failed parsing workflow json '{}': {err}",
                    path.display()
                ))
            })
        }
    }

    fn validate(
        workflow: &WorkflowDefinition,
        file: &Path,
        config: &WorkflowsConfig,
    ) -> Result<()> {
        if workflow.name.trim().is_empty() {
            return Err(Error::Validation(format!(
                "workflow '{}' must define non-empty name",
                file.display()
            )));
        }
        if workflow.steps.is_empty() {
            return Err(Error::Validation(format!(
                "workflow '{}' must define at least one step",
                workflow.name
            )));
        }

        if let Some(max_steps) = config.max_steps_per_run {
            if workflow.steps.len() > max_steps {
                return Err(Error::Validation(format!(
                    "workflow '{}' has {} steps which exceeds max_steps_per_run ({max_steps})",
                    workflow.name,
                    workflow.steps.len()
                )));
            }
        }

        let mut ids = HashSet::new();
        for step in &workflow.steps {
            if step.id.trim().is_empty() {
                return Err(Error::Validation(format!(
                    "workflow '{}' contains a step with empty id",
                    workflow.name
                )));
            }
            if !ids.insert(step.id.clone()) {
                return Err(Error::Validation(format!(
                    "workflow '{}' contains duplicate step id '{}'",
                    workflow.name, step.id
                )));
            }
            if step.kind == WorkflowStepKind::Condition {
                let path_present = step
                    .config
                    .get("path")
                    .and_then(|value| value.as_str())
                    .is_some();
                if !path_present {
                    return Err(Error::Validation(format!(
                        "workflow '{}' condition step '{}' must define config.path",
                        workflow.name, step.id
                    )));
                }
            }
        }

        for step in &workflow.steps {
            for target in [&step.next, &step.on_success, &step.on_failure]
                .into_iter()
                .flatten()
            {
                if !ids.contains(target) {
                    return Err(Error::Validation(format!(
                        "workflow '{}' step '{}' references unknown target step '{}'",
                        workflow.name, step.id, target
                    )));
                }
            }
        }

        if workflow.entrypoints.is_empty() {
            return Err(Error::Validation(format!(
                "workflow '{}' must define at least one entrypoint",
                workflow.name
            )));
        }

        for (name, entrypoint) in &workflow.entrypoints {
            if name.trim().is_empty() {
                return Err(Error::Validation(format!(
                    "workflow '{}' has empty entrypoint name",
                    workflow.name
                )));
            }
            if entrypoint.step.trim().is_empty() || !ids.contains(&entrypoint.step) {
                return Err(Error::Validation(format!(
                    "workflow '{}' entrypoint '{}' references unknown step '{}'",
                    workflow.name, name, entrypoint.step
                )));
            }
        }

        Ok(())
    }

    pub fn load(config: &WorkflowsConfig, work_dir: &Path) -> Result<WorkflowRegistry> {
        let mut registry = WorkflowRegistry::new();
        let files = Self::discover_files(config, work_dir);
        for file in files {
            let workflow = Self::parse_file(&file)?;
            Self::validate(&workflow, &file, config)?;
            registry.register(workflow);
        }
        Ok(registry)
    }
}
