mod bridge;
mod cli;
mod renderer;
mod repl;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use jsonschema::JSONSchema;
use rustic_ai_core::config::{ConfigChange, ConfigManager, ConfigPath, ConfigScope};
use serde::{Deserialize, Serialize};

fn handle_session_command(
    app: &rustic_ai_core::RusticAI,
    command: cli::SessionCommand,
) -> rustic_ai_core::Result<()> {
    let runtime = tokio::runtime::Runtime::new().map_err(|err| {
        rustic_ai_core::Error::Config(format!("failed to create tokio runtime: {err}"))
    })?;

    match command {
        cli::SessionCommand::List => {
            let sessions = runtime.block_on(app.session_manager().list_sessions(None))?;
            println!("Sessions:");
            for session in sessions {
                println!(
                    "- {} (agent: {}, created: {})",
                    session.id, session.agent_name, session.created_at
                );
            }
        }
        cli::SessionCommand::Create { agent } => {
            let session_id = runtime.block_on(
                app.session_manager()
                    .create_session(agent.as_deref().unwrap_or("default")),
            )?;
            println!("Created session: {}", session_id);
        }
        cli::SessionCommand::Continue { id } => {
            let session_id = uuid::Uuid::parse_str(&id).map_err(|err| {
                rustic_ai_core::Error::Config(format!("invalid session id '{id}': {err}"))
            })?;
            let session = runtime.block_on(app.session_manager().get_session(session_id))?;
            if let Some(session) = session {
                println!("Session: {} (agent: {})", session_id, session.agent_name);
            } else {
                println!("Session not found: {}", id);
            }
        }
        cli::SessionCommand::Delete { id } => {
            let session_id = uuid::Uuid::parse_str(&id).map_err(|err| {
                rustic_ai_core::Error::Config(format!("invalid session id '{id}': {err}"))
            })?;
            runtime.block_on(app.session_manager().delete_session(session_id))?;
            println!("Deleted session: {}", id);
        }
    }

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rustic-ai-cli failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> rustic_ai_core::Result<()> {
    let args = cli::Cli::parse_args();
    let config_path = PathBuf::from(&args.config);

    if let Some(command) = args.command {
        match command {
            cli::Command::ValidateConfig { schema, strict } => {
                validate_config_against_schema(&config_path, &PathBuf::from(schema), strict)?;
                let loaded_config = rustic_ai_core::config::load_from_file(&config_path)?;
                rustic_ai_core::config::validate_config(&loaded_config)?;
                println!("Config is valid against schema and runtime checks.");
                return Ok(());
            }
            cli::Command::Config { command, output } => {
                if let Err(error) = handle_config_command(&config_path, command, output) {
                    match output {
                        cli::OutputFormat::Text => return Err(error),
                        cli::OutputFormat::Json => {
                            print_json_error_envelope("config", &error)?;
                            std::process::exit(1);
                        }
                    }
                }
                return Ok(());
            }
            other => {
                let app = rustic_ai_core::RusticAI::from_config_path(&config_path)?;
                match other {
                    cli::Command::Discover => {
                        println!("Discovered rules:");
                        for rule in &app.config().rules.discovered_rules {
                            println!("- {} [{:?}]", rule.path, rule.scope);
                            if let Some(description) = &rule.description {
                                println!("  description: {description}");
                            }
                            if !rule.globs.is_empty() {
                                println!("  globs: {}", rule.globs.join(", "));
                            }
                            if !rule.topics.is_empty() {
                                println!("  topics: {}", rule.topics.join(", "));
                            }
                            println!("  always_apply: {}", rule.always_apply);
                        }
                        return Ok(());
                    }
                    cli::Command::Topics => {
                        let runtime = tokio::runtime::Runtime::new().map_err(|err| {
                            rustic_ai_core::Error::Config(format!(
                                "failed to create tokio runtime: {err}"
                            ))
                        })?;

                        let session_id = if let Some(value) = args.session_id {
                            uuid::Uuid::parse_str(&value).map_err(|err| {
                                rustic_ai_core::Error::Config(format!(
                                    "invalid --session-id '{value}': {err}"
                                ))
                            })?
                        } else {
                            runtime.block_on(app.session_manager().create_session("default"))?
                        };

                        let topics = runtime
                            .block_on(app.session_manager().get_session_topics(session_id))?
                            .unwrap_or_default();
                        println!("Session: {session_id}");
                        println!("Topics: {}", topics.join(", "));
                        return Ok(());
                    }
                    cli::Command::Session { command } => {
                        handle_session_command(&app, command)?;
                        return Ok(());
                    }
                    cli::Command::Chat { agent, output } => {
                        let app = std::sync::Arc::new(app);
                        let repl = repl::Repl::new(app, agent.clone(), output);
                        let runtime = tokio::runtime::Runtime::new().map_err(|err| {
                            rustic_ai_core::Error::Config(format!(
                                "failed to create tokio runtime: {err}"
                            ))
                        })?;
                        runtime.block_on(repl.run())?;
                        return Ok(());
                    }
                    _ => unreachable!("command variant handled earlier"),
                }
            }
        }
    }

    let app = rustic_ai_core::RusticAI::from_config_path(&config_path)?;
    println!("rustic-ai-cli initialized in {:?} mode", app.config().mode);
    Ok(())
}

fn handle_config_command(
    config_path: &Path,
    command: cli::ConfigCommand,
    output: cli::OutputFormat,
) -> rustic_ai_core::Result<()> {
    let runtime = tokio::runtime::Runtime::new().map_err(|err| {
        rustic_ai_core::Error::Config(format!("failed to create tokio runtime: {err}"))
    })?;

    let manager = runtime.block_on(build_config_manager(config_path))?;

    match command {
        cli::ConfigCommand::Snapshot => {
            let snapshot = runtime.block_on(manager.snapshot())?;
            match output {
                cli::OutputFormat::Text => {
                    println!("Version: {}", snapshot.version);
                    println!("Path: {}", snapshot.path.display());
                    println!("{}", serde_json::to_string_pretty(&snapshot.config)?);
                }
                cli::OutputFormat::Json => {
                    print_json_envelope(
                        "config.snapshot",
                        &SnapshotData {
                            version: snapshot.version,
                            path: snapshot.path.display().to_string(),
                            config: snapshot.config,
                        },
                    )?;
                }
            }
        }
        cli::ConfigCommand::Get {
            path,
            scope,
            explain,
        } => {
            let parsed = ConfigPath::from_str(&path)?;
            match scope {
                cli::ConfigReadScope::Effective => {
                    let resolved =
                        runtime.block_on(manager.get_effective_value_with_source(&parsed))?;
                    match output {
                        cli::OutputFormat::Text => {
                            if explain {
                                println!("Path: {}", parsed);
                                println!("Source: {:?}", resolved.source);
                                println!(
                                    "Project: {}",
                                    resolved
                                        .project
                                        .as_ref()
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "<not set>".to_owned())
                                );
                                println!(
                                    "Global: {}",
                                    resolved
                                        .global
                                        .as_ref()
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "<not set>".to_owned())
                                );
                            }
                            println!("Effective: {}", serde_json::to_string(&resolved.effective)?);
                        }
                        cli::OutputFormat::Json => {
                            print_json_envelope(
                                "config.get",
                                &EffectiveGetData {
                                    path: parsed.to_string(),
                                    source: resolved.source,
                                    effective: resolved.effective,
                                    project: resolved.project,
                                    global: resolved.global,
                                },
                            )?;
                        }
                    }
                }
                cli::ConfigReadScope::Project => {
                    let value =
                        runtime.block_on(manager.get_value(ConfigScope::Project, &parsed))?;
                    print_value_by_output(&parsed, ConfigScope::Project, &value, output)?;
                }
                cli::ConfigReadScope::Global => {
                    let value =
                        runtime.block_on(manager.get_value(ConfigScope::Global, &parsed))?;
                    print_value_by_output(&parsed, ConfigScope::Global, &value, output)?;
                }
            }
        }
        cli::ConfigCommand::Set {
            scope,
            path,
            value_json,
            expected_version,
        } => {
            let parsed = ConfigPath::from_str(&path)?;
            let value: serde_json::Value = serde_json::from_str(&value_json).map_err(|err| {
                rustic_ai_core::Error::Validation(format!(
                    "--value-json must be valid JSON value: {err}"
                ))
            })?;
            let target_scope = match scope {
                cli::ConfigWriteScope::Project => ConfigScope::Project,
                cli::ConfigWriteScope::Global => ConfigScope::Global,
            };

            let snapshot = runtime.block_on(manager.set_value(
                target_scope,
                parsed,
                value,
                expected_version,
            ))?;
            print_mutation_result("updated", snapshot.version, output)?;
        }
        cli::ConfigCommand::Unset {
            scope,
            path,
            expected_version,
        } => {
            let parsed = ConfigPath::from_str(&path)?;
            let target_scope = match scope {
                cli::ConfigWriteScope::Project => ConfigScope::Project,
                cli::ConfigWriteScope::Global => ConfigScope::Global,
            };
            let snapshot =
                runtime.block_on(manager.unset_value(target_scope, parsed, expected_version))?;
            print_mutation_result("unset", snapshot.version, output)?;
        }
        cli::ConfigCommand::Patch {
            file,
            expected_version,
        } => {
            let payload = std::fs::read_to_string(&file).map_err(|err| {
                rustic_ai_core::Error::Config(format!("failed to read patch file '{file}': {err}"))
            })?;
            let patch_input: Vec<PatchInput> = serde_json::from_str(&payload).map_err(|err| {
                rustic_ai_core::Error::Validation(format!(
                    "patch file must be valid JSON array of patch items: {err}"
                ))
            })?;
            let mut changes = Vec::with_capacity(patch_input.len());
            for item in patch_input {
                changes.push(ConfigChange {
                    scope: item.scope,
                    path: ConfigPath::from_str(&item.path)?,
                    value: item.value,
                });
            }

            let snapshot = runtime.block_on(manager.patch(changes, expected_version))?;
            print_mutation_result("patched", snapshot.version, output)?;
        }
    }

    Ok(())
}

fn print_value_by_output(
    path: &ConfigPath,
    scope: ConfigScope,
    value: &serde_json::Value,
    output: cli::OutputFormat,
) -> rustic_ai_core::Result<()> {
    match output {
        cli::OutputFormat::Text => {
            println!("{}", serde_json::to_string(value)?);
        }
        cli::OutputFormat::Json => {
            print_json_envelope(
                "config.get",
                &ScopedGetData {
                    path: path.to_string(),
                    scope,
                    value: value.clone(),
                },
            )?;
        }
    }
    Ok(())
}

fn print_mutation_result(
    action: &str,
    version: u64,
    output: cli::OutputFormat,
) -> rustic_ai_core::Result<()> {
    match output {
        cli::OutputFormat::Text => {
            println!("{}: version {}", action, version);
        }
        cli::OutputFormat::Json => {
            print_json_envelope(
                "config.mutation",
                &MutationData {
                    action: action.to_owned(),
                    version,
                },
            )?;
        }
    }
    Ok(())
}

fn print_json_envelope<T: Serialize>(command: &str, data: &T) -> rustic_ai_core::Result<()> {
    let envelope = CliJsonEnvelope {
        schema: "rustic-ai-cli/config-output/v1",
        status: "ok",
        command,
        data,
    };
    println!("{}", serde_json::to_string(&envelope)?);
    Ok(())
}

fn print_json_error_envelope(
    command: &str,
    error: &rustic_ai_core::Error,
) -> rustic_ai_core::Result<()> {
    let envelope = CliJsonErrorEnvelope {
        schema: "rustic-ai-cli/config-output/v1",
        status: "error",
        command,
        error: CliJsonErrorPayload {
            code: error_code(error),
            message: error.to_string(),
            details: None,
        },
    };
    println!("{}", serde_json::to_string(&envelope)?);
    Ok(())
}

fn error_code(error: &rustic_ai_core::Error) -> &'static str {
    match error {
        rustic_ai_core::Error::Config(_) => "config_error",
        rustic_ai_core::Error::Validation(_) => "validation_error",
        rustic_ai_core::Error::NotFound(_) => "not_found",
        rustic_ai_core::Error::Provider(_) => "provider_error",
        rustic_ai_core::Error::Tool(_) => "tool_error",
        rustic_ai_core::Error::Storage(_) => "storage_error",
        rustic_ai_core::Error::Io(_) => "io_error",
        rustic_ai_core::Error::Sqlx(_) => "sqlx_error",
        rustic_ai_core::Error::TomlParse(_) => "toml_parse_error",
        rustic_ai_core::Error::TomlSerialize(_) => "toml_serialize_error",
        rustic_ai_core::Error::Json(_) => "json_error",
    }
}

#[derive(Debug, Serialize)]
struct CliJsonEnvelope<'a, T: Serialize> {
    schema: &'a str,
    status: &'a str,
    command: &'a str,
    data: &'a T,
}

#[derive(Debug, Serialize)]
struct CliJsonErrorEnvelope<'a> {
    schema: &'a str,
    status: &'a str,
    command: &'a str,
    error: CliJsonErrorPayload,
}

#[derive(Debug, Serialize)]
struct CliJsonErrorPayload {
    code: &'static str,
    message: String,
    details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SnapshotData {
    version: u64,
    path: String,
    config: rustic_ai_core::Config,
}

#[derive(Debug, Serialize)]
struct EffectiveGetData {
    path: String,
    source: rustic_ai_core::config::ConfigValueSource,
    effective: serde_json::Value,
    project: Option<serde_json::Value>,
    global: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ScopedGetData {
    path: String,
    scope: ConfigScope,
    value: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct MutationData {
    action: String,
    version: u64,
}

#[derive(Debug, Deserialize)]
struct PatchInput {
    scope: ConfigScope,
    path: String,
    value: serde_json::Value,
}

async fn build_config_manager(config_path: &Path) -> rustic_ai_core::Result<ConfigManager> {
    let work_dir = std::env::current_dir().map_err(|err| {
        rustic_ai_core::Error::Config(format!("failed to resolve current dir: {err}"))
    })?;

    let project_config = if config_path.exists() {
        rustic_ai_core::config::load_from_file(config_path)?
    } else {
        rustic_ai_core::Config::default()
    };
    let storage_paths =
        rustic_ai_core::storage::paths::StoragePaths::resolve(&work_dir, &project_config);

    ConfigManager::load(config_path.to_path_buf(), storage_paths.global_settings).await
}

fn validate_config_against_schema(
    config_path: &Path,
    schema_path: &Path,
    strict: bool,
) -> rustic_ai_core::Result<()> {
    let config_raw = std::fs::read_to_string(config_path).map_err(|err| {
        rustic_ai_core::Error::Config(format!(
            "failed to read config file '{}': {err}",
            config_path.display()
        ))
    })?;
    let schema_raw = std::fs::read_to_string(schema_path).map_err(|err| {
        rustic_ai_core::Error::Config(format!(
            "failed to read schema file '{}': {err}",
            schema_path.display()
        ))
    })?;

    let config_json: serde_json::Value = serde_json::from_str(&config_raw).map_err(|err| {
        rustic_ai_core::Error::Config(format!(
            "config is not valid JSON '{}': {err}",
            config_path.display()
        ))
    })?;
    let schema_json: serde_json::Value = serde_json::from_str(&schema_raw).map_err(|err| {
        rustic_ai_core::Error::Config(format!(
            "schema is not valid JSON '{}': {err}",
            schema_path.display()
        ))
    })?;

    let compiled = JSONSchema::options().compile(&schema_json).map_err(|err| {
        rustic_ai_core::Error::Config(format!("failed to compile JSON schema: {err}"))
    })?;

    if let Err(errors) = compiled.validate(&config_json) {
        let details = errors
            .map(|err| err.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(rustic_ai_core::Error::Validation(format!(
            "schema validation failed: {details}"
        )));
    }

    if strict {
        validate_config_strict(&config_json)?;
    }

    Ok(())
}

fn validate_config_strict(config_json: &serde_json::Value) -> rustic_ai_core::Result<()> {
    let Some(config_obj) = config_json.as_object() else {
        return Err(rustic_ai_core::Error::Validation(
            "strict validation requires config root to be a JSON object".to_owned(),
        ));
    };

    let providers = config_obj
        .get("providers")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            rustic_ai_core::Error::Validation(
                "strict validation requires 'providers' to be an array".to_owned(),
            )
        })?;

    for (index, provider) in providers.iter().enumerate() {
        let Some(provider_obj) = provider.as_object() else {
            return Err(rustic_ai_core::Error::Validation(format!(
                "strict validation requires providers[{index}] to be an object"
            )));
        };

        for required in ["name", "provider_type", "auth_mode"] {
            let value = provider_obj.get(required).ok_or_else(|| {
                rustic_ai_core::Error::Validation(format!(
                    "strict validation requires providers[{index}].{required} to be set"
                ))
            })?;

            if value.is_null() {
                return Err(rustic_ai_core::Error::Validation(format!(
                    "strict validation does not allow null for providers[{index}].{required}"
                )));
            }

            if value.as_str().map(|s| s.trim().is_empty()).unwrap_or(false) {
                return Err(rustic_ai_core::Error::Validation(format!(
                    "strict validation does not allow empty values for providers[{index}].{required}"
                )));
            }
        }

        let provider_type = provider_obj
            .get("provider_type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        if provider_type == "open_ai" {
            for required in ["model", "api_key_env", "base_url"] {
                let value = provider_obj.get(required).ok_or_else(|| {
                    rustic_ai_core::Error::Validation(format!(
                        "strict validation requires providers[{index}].{required} for open_ai"
                    ))
                })?;

                if value.is_null() {
                    return Err(rustic_ai_core::Error::Validation(format!(
                        "strict validation does not allow null for providers[{index}].{required}"
                    )));
                }

                if value.as_str().map(|s| s.trim().is_empty()).unwrap_or(false) {
                    return Err(rustic_ai_core::Error::Validation(format!(
                        "strict validation does not allow empty values for providers[{index}].{required}"
                    )));
                }
            }
        }
    }

    let summarization_provider = config_obj
        .get("summarization")
        .and_then(|value| value.as_object())
        .and_then(|value| value.get("provider_name"))
        .ok_or_else(|| {
            rustic_ai_core::Error::Validation(
                "strict validation requires summarization.provider_name to be set".to_owned(),
            )
        })?;

    if summarization_provider.is_null()
        || summarization_provider
            .as_str()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(rustic_ai_core::Error::Validation(
            "strict validation does not allow null/empty summarization.provider_name".to_owned(),
        ));
    }

    Ok(())
}
