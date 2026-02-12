use chrono::Utc;

use crate::config::schema::AgentConfig;
use crate::conversation::session_manager::SessionManager;
use crate::error::Result;
use crate::storage::model::{Todo, TodoMetadata, TodoPriority, TodoStatus};

struct TodoCreationSpec {
    child_titles: Vec<String>,
    parent_title: String,
    parent_description: Option<String>,
    parent_priority: TodoPriority,
    parent_tags: Vec<String>,
    child_tags: Vec<String>,
}

pub async fn auto_create_todos_from_response(
    session_manager: &SessionManager,
    config: &AgentConfig,
    session_id: uuid::Uuid,
    response: &str,
) -> Result<()> {
    if !config.auto_create_todos {
        return Ok(());
    }

    let todo_items = parse_response_todo_items(response);
    if todo_items.is_empty() {
        return Ok(());
    }
    let item_count = todo_items.len();

    create_todo_hierarchy(
        session_manager,
        config,
        session_id,
        TodoCreationSpec {
            child_titles: todo_items,
            parent_title: format!("Session TODOs ({item_count})"),
            parent_description: Some("Auto-generated TODOs from agent response".to_string()),
            parent_priority: TodoPriority::Medium,
            parent_tags: vec!["auto-generated".to_string()],
            child_tags: vec!["auto-generated".to_string()],
        },
    )
    .await
}

pub async fn auto_create_todos_from_input(
    session_manager: &SessionManager,
    config: &AgentConfig,
    session_id: uuid::Uuid,
    input: &str,
) -> Result<()> {
    if !config.auto_create_todos {
        return Ok(());
    }

    let tasks = parse_input_tasks(input);
    if tasks.len() < 2 {
        return Ok(());
    }

    create_todo_hierarchy(
        session_manager,
        config,
        session_id,
        TodoCreationSpec {
            child_titles: tasks.clone(),
            parent_title: format!("User request with {} tasks", tasks.len()),
            parent_description: Some(input.to_string()),
            parent_priority: TodoPriority::High,
            parent_tags: vec!["auto-generated".to_string(), "multi-step".to_string()],
            child_tags: vec!["auto-generated".to_string(), "input-task".to_string()],
        },
    )
    .await
}

fn parse_response_todo_items(response: &str) -> Vec<String> {
    let mut todo_items = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        let title = if trimmed.starts_with("TODO:") || trimmed.starts_with("todo:") {
            trimmed[5..].trim().to_string()
        } else if trimmed.starts_with("- [ ]") {
            trimmed[4..].trim().to_string()
        } else {
            continue;
        };

        if !title.is_empty() {
            todo_items.push(title);
        }
    }

    todo_items
}

fn parse_input_tasks(input: &str) -> Vec<String> {
    let mut tasks = Vec::new();

    for segment in input.split('\n') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }

        let normalized = trimmed
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == ' ')
            .trim();
        if normalized.is_empty() {
            continue;
        }

        if normalized.contains(" and ") {
            for part in normalized.split(" and ") {
                let part = part.trim().trim_end_matches('.');
                if !part.is_empty() {
                    tasks.push(part.to_string());
                }
            }
            continue;
        }

        if normalized.contains(',') {
            for part in normalized.split(',') {
                let part = part.trim().trim_end_matches('.');
                if !part.is_empty() {
                    tasks.push(part.to_string());
                }
            }
            continue;
        }

        tasks.push(normalized.trim_end_matches('.').to_string());
    }

    tasks.sort();
    tasks.dedup();
    tasks
}

async fn create_todo_hierarchy(
    session_manager: &SessionManager,
    config: &AgentConfig,
    session_id: uuid::Uuid,
    spec: TodoCreationSpec,
) -> Result<()> {
    let TodoCreationSpec {
        child_titles,
        parent_title,
        parent_description,
        parent_priority,
        parent_tags,
        child_tags,
    } = spec;

    let project_id = session_manager.project_profile().map(|p| p.name.clone());

    let parent_id = if config.todo_project_scope && project_id.is_some() {
        let parent_id = uuid::Uuid::new_v4();
        let parent_todo = Todo {
            id: parent_id,
            project_id: project_id.clone(),
            session_id,
            parent_id: None,
            title: parent_title,
            description: parent_description,
            status: TodoStatus::Todo,
            priority: parent_priority,
            tags: parent_tags,
            metadata: TodoMetadata::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        session_manager.create_todo(&parent_todo).await?;
        Some(parent_id)
    } else {
        None
    };

    for title in child_titles {
        let todo = Todo {
            id: uuid::Uuid::new_v4(),
            project_id: if config.todo_project_scope {
                project_id.clone()
            } else {
                None
            },
            session_id,
            parent_id,
            title,
            description: None,
            status: TodoStatus::Todo,
            priority: TodoPriority::Medium,
            tags: child_tags.clone(),
            metadata: TodoMetadata::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        session_manager.create_todo(&todo).await?;
    }

    Ok(())
}
