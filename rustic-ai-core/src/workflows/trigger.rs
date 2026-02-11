use chrono::{DateTime, Utc};
use cron::Schedule;
use std::collections::HashSet;
use std::str::FromStr;

use crate::workflows::registry::WorkflowRegistry;

#[derive(Debug, Clone)]
pub enum WorkflowTriggerReason {
    Event { event_name: String },
    Cron { expression: String },
}

#[derive(Debug, Clone)]
pub struct WorkflowTriggerMatch {
    pub workflow_name: String,
    pub entrypoint: String,
    pub reason: WorkflowTriggerReason,
}

#[derive(Debug, Clone)]
pub struct WorkflowTriggerEngine {
    last_cron_check: DateTime<Utc>,
}

impl WorkflowTriggerEngine {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            last_cron_check: now,
        }
    }

    pub fn due_cron(
        &mut self,
        workflows: &WorkflowRegistry,
        now: DateTime<Utc>,
    ) -> Vec<WorkflowTriggerMatch> {
        let from = self.last_cron_check;
        self.last_cron_check = now;

        let mut seen = HashSet::new();
        let mut due = Vec::new();

        for workflow_name in workflows.list() {
            let Some(workflow) = workflows.get(&workflow_name) else {
                continue;
            };

            for (entrypoint_name, entrypoint) in &workflow.entrypoints {
                for expr in &entrypoint.triggers.cron {
                    let parsed = match Schedule::from_str(expr) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };

                    let matched = parsed.after(&from).take(1).any(|next| next <= now);
                    if !matched {
                        continue;
                    }

                    let key = format!("{}::{}::{}", workflow.name, entrypoint_name, expr);
                    if seen.insert(key) {
                        due.push(WorkflowTriggerMatch {
                            workflow_name: workflow.name.clone(),
                            entrypoint: entrypoint_name.clone(),
                            reason: WorkflowTriggerReason::Cron {
                                expression: expr.clone(),
                            },
                        });
                    }
                }
            }
        }

        due
    }

    pub fn for_event(workflows: &WorkflowRegistry, event_name: &str) -> Vec<WorkflowTriggerMatch> {
        let event_name = event_name.trim().to_ascii_lowercase();
        if event_name.is_empty() {
            return Vec::new();
        }

        let mut due = Vec::new();
        for workflow_name in workflows.list() {
            let Some(workflow) = workflows.get(&workflow_name) else {
                continue;
            };
            for (entrypoint_name, entrypoint) in &workflow.entrypoints {
                if entrypoint
                    .triggers
                    .events
                    .iter()
                    .any(|event| event.trim().eq_ignore_ascii_case(&event_name))
                {
                    due.push(WorkflowTriggerMatch {
                        workflow_name: workflow.name.clone(),
                        entrypoint: entrypoint_name.clone(),
                        reason: WorkflowTriggerReason::Event {
                            event_name: event_name.clone(),
                        },
                    });
                }
            }
        }

        due
    }
}
