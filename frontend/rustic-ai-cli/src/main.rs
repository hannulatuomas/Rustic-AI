mod bridge;
mod cli;
mod renderer;
mod repl;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use jsonschema::JSONSchema;
use rustic_ai_core::auth::{CredentialStore, SubscriptionAuthManager};
use rustic_ai_core::config::schema::{AuthMode, ProviderType};
use rustic_ai_core::config::{ConfigChange, ConfigManager, ConfigPath, ConfigScope};
use rustic_ai_core::providers::auth_capabilities::{auth_mode_name, supported_auth_mode_names};
use serde::{Deserialize, Serialize};

fn open_url_in_browser(url: &str) -> rustic_ai_core::Result<()> {
    #[cfg(target_os = "linux")]
    let command = ("xdg-open", vec![url]);

    #[cfg(target_os = "macos")]
    let command = ("open", vec![url]);

    #[cfg(target_os = "windows")]
    let command = ("cmd", vec!["/C", "start", "", url]);

    let status = std::process::Command::new(command.0)
        .args(command.1)
        .status()
        .map_err(|err| {
            rustic_ai_core::Error::Config(format!("failed to open browser automatically: {err}"))
        })?;

    if !status.success() {
        return Err(rustic_ai_core::Error::Config(
            "failed to open browser automatically".to_owned(),
        ));
    }

    Ok(())
}

fn resolve_provider<'a>(
    config: &'a rustic_ai_core::Config,
    provider_name: &str,
) -> rustic_ai_core::Result<&'a rustic_ai_core::config::schema::ProviderConfig> {
    config
        .providers
        .iter()
        .find(|provider| provider.name == provider_name)
        .ok_or_else(|| {
            rustic_ai_core::Error::Config(format!(
                "provider '{}' not found in config",
                provider_name
            ))
        })
}

fn provider_type_name(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::OpenAi => "open_ai",
        ProviderType::Anthropic => "anthropic",
        ProviderType::Grok => "grok",
        ProviderType::Google => "google",
        ProviderType::ZAi => "z_ai",
        ProviderType::Ollama => "ollama",
        ProviderType::Custom => "custom",
    }
}

fn handle_auth_command(
    config_path: &Path,
    command: cli::AuthCommand,
) -> rustic_ai_core::Result<()> {
    let runtime = tokio::runtime::Runtime::new().map_err(|err| {
        rustic_ai_core::Error::Config(format!("failed to create tokio runtime: {err}"))
    })?;

    let config = rustic_ai_core::config::load_from_file(config_path)?;
    let work_dir = std::env::current_dir().map_err(|err| {
        rustic_ai_core::Error::Config(format!("failed to resolve current dir: {err}"))
    })?;
    let auth_store_path = rustic_ai_core::auth::resolve_auth_store_path(&config, &work_dir);

    match command {
        cli::AuthCommand::Methods => {
            if config.providers.is_empty() {
                println!("No providers configured.");
                return Ok(());
            }

            println!("Provider auth methods:");
            for provider in &config.providers {
                let supported = supported_auth_mode_names(&provider.provider_type).join(", ");
                println!(
                    "- {} (type: {}, configured: {}, supported: {})",
                    provider.name,
                    provider_type_name(&provider.provider_type),
                    auth_mode_name(&provider.auth_mode),
                    supported
                );
            }
            Ok(())
        }
        cli::AuthCommand::List => {
            let store = CredentialStore::new(auth_store_path);
            let entries = runtime.block_on(store.list_summaries())?;
            if entries.is_empty() {
                println!("No stored subscription credentials.");
                return Ok(());
            }

            println!("Stored subscription credentials:");
            for entry in entries {
                println!(
                    "- {} (token_type: {}, expires_at: {}, scopes: {})",
                    entry.provider_name,
                    entry.token_type,
                    entry
                        .expires_at_epoch_secs
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "never".to_owned()),
                    if entry.scopes.is_empty() {
                        "<none>".to_owned()
                    } else {
                        entry.scopes.join(",")
                    }
                );
            }
            Ok(())
        }
        cli::AuthCommand::Logout { provider } => {
            let store = CredentialStore::new(auth_store_path);
            let removed = runtime.block_on(store.remove(&provider))?;
            if removed {
                println!("Removed stored credential for provider '{provider}'.");
            } else {
                println!("No stored credential found for provider '{provider}'.");
            }
            Ok(())
        }
        cli::AuthCommand::Connect {
            provider,
            method,
            no_browser,
            timeout_secs,
        } => {
            let provider_config = resolve_provider(&config, &provider)?;
            let supported = supported_auth_mode_names(&provider_config.provider_type);

            if !supported.contains(&"subscription") {
                return Err(rustic_ai_core::Error::Config(format!(
                    "provider '{}' (type: {}) does not support subscription auth; supported auth modes: {}",
                    provider,
                    provider_type_name(&provider_config.provider_type),
                    supported.join(", ")
                )));
            }

            if provider_config.auth_mode != AuthMode::Subscription {
                return Err(rustic_ai_core::Error::Config(format!(
                    "provider '{}' is configured with auth_mode '{}' but 'auth connect' requires auth_mode 'subscription'",
                    provider,
                    auth_mode_name(&provider_config.auth_mode)
                )));
            }

            let manager =
                SubscriptionAuthManager::from_provider_config(provider_config, auth_store_path)?;

            match method {
                cli::AuthMethod::Browser => {
                    let request = manager.build_browser_authorization_request()?;
                    println!(
                        "Open this URL to authenticate:\n{}",
                        request.authorization_url
                    );
                    if !no_browser {
                        if let Err(err) = open_url_in_browser(&request.authorization_url) {
                            eprintln!("Browser auto-open failed: {err}");
                            eprintln!("Continue manually by opening the URL above.");
                        }
                    }

                    println!(
                        "Waiting for OAuth callback on http://{}:{}/callback ...",
                        manager.oauth_config().redirect_host,
                        manager.oauth_config().redirect_port
                    );

                    let code = runtime.block_on(manager.await_browser_callback_code(
                        &request.expected_state,
                        std::time::Duration::from_secs(timeout_secs),
                    ))?;

                    let credential = runtime.block_on(manager.exchange_authorization_code(
                        &code,
                        &request.code_verifier,
                        &request.redirect_uri,
                    ))?;

                    runtime.block_on(manager.save_credential(credential))?;
                    println!("Authentication successful for provider '{provider}'.");
                    Ok(())
                }
                cli::AuthMethod::Headless => {
                    let start = runtime.block_on(manager.start_device_authorization())?;
                    println!("Device authentication started.");
                    println!("User code: {}", start.user_code);
                    println!("Verification URL: {}", start.verification_uri);
                    if let Some(url) = &start.verification_uri_complete {
                        println!("Complete URL: {}", url);
                        if !no_browser {
                            if let Err(err) = open_url_in_browser(url) {
                                eprintln!("Browser auto-open failed: {err}");
                            }
                        }
                    }
                    println!("Waiting for authorization confirmation...");

                    let credential = runtime.block_on(manager.poll_device_authorization(&start))?;
                    runtime.block_on(manager.save_credential(credential))?;
                    println!("Authentication successful for provider '{provider}'.");
                    Ok(())
                }
            }
        }
    }
}

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
            cli::Command::Auth { command } => {
                handle_auth_command(&config_path, command)?;
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
                    cli::Command::Agents => {
                        let names = app.runtime().agents.list_agents();
                        if names.is_empty() {
                            println!("No agents configured.");
                            return Ok(());
                        }

                        println!("Configured agents:");
                        for name in names {
                            if let Some(config) = app.runtime().agents.get_agent_config(&name) {
                                println!(
                                    "- {} (provider={}, permission_mode={:?}, allow_sub_agent_calls={}, max_sub_agent_depth={}, sub_agent_context_window_size={}, sub_agent_max_context_tokens={})",
                                    config.name,
                                    config.provider,
                                    config.permission_mode,
                                    config.allow_sub_agent_calls,
                                    config
                                        .max_sub_agent_depth
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "<default>".to_owned()),
                                    config
                                        .sub_agent_context_window_size
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "<default>".to_owned()),
                                    config
                                        .sub_agent_max_context_tokens
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "<default>".to_owned())
                                );
                                if config.tools.is_empty() {
                                    println!("  tools: <none>");
                                } else {
                                    println!("  tools: {}", config.tools.join(", "));
                                }
                                if config.skills.is_empty() {
                                    println!("  skills: <none>");
                                } else {
                                    println!("  skills: {}", config.skills.join(", "));
                                }
                            }
                        }
                        return Ok(());
                    }
                    cli::Command::Index { command } => {
                        let runtime = tokio::runtime::Runtime::new().map_err(|err| {
                            rustic_ai_core::Error::Config(format!(
                                "failed to create tokio runtime: {err}"
                            ))
                        })?;

                        match command {
                            cli::IndexCommand::Status => {
                                let config = app.config();
                                let retrieval = &config.retrieval;
                                let sqlite = &config.storage.sqlite;

                                println!("Index/Retrieval status:");
                                println!(
                                    "- features: indexing={}, vector={}, rag={}, retrieval={}",
                                    config.features.indexing_enabled,
                                    config.features.vector_enabled,
                                    config.features.rag_enabled,
                                    retrieval.enabled
                                );
                                println!(
                                    "- retrieval limits: keyword_top_k={}, vector_top_k={}, max_snippets={}, snippet_chars={}, rag_prompt_token_budget={}",
                                    retrieval.keyword_top_k,
                                    retrieval.vector_top_k,
                                    retrieval.max_snippets,
                                    retrieval.max_snippet_chars,
                                    retrieval.rag_prompt_token_budget
                                );
                                println!(
                                    "- ranking/context: min_vector_score={}, context_expansion_lines={}, recency_weight={}, importance_weight={}",
                                    retrieval.min_vector_score,
                                    retrieval.context_expansion_lines,
                                    retrieval.ranking_recency_weight,
                                    retrieval.ranking_importance_weight
                                );
                                println!(
                                    "- embedding: backend={:?}, model={}, base_url={}, api_key_env={}",
                                    retrieval.embedding_backend,
                                    retrieval.embedding_model.as_deref().unwrap_or("<unset>"),
                                    retrieval
                                        .embedding_base_url
                                        .as_deref()
                                        .unwrap_or("<unset>"),
                                    retrieval
                                        .embedding_api_key_env
                                        .as_deref()
                                        .unwrap_or("<unset>")
                                );

                                if let Some(env_name) = retrieval.embedding_api_key_env.as_deref() {
                                    let is_set = std::env::var(env_name)
                                        .map(|value| !value.trim().is_empty())
                                        .unwrap_or(false);
                                    println!("- embedding_api_key_present: {}", is_set);
                                }

                                if config.storage.backend
                                    == rustic_ai_core::config::schema::StorageBackendKind::Sqlite
                                {
                                    println!(
                                        "- sqlite_vector_extension: enabled={}, strict={}, path={}, entrypoint={}",
                                        sqlite.vector_extension_enabled,
                                        sqlite.vector_extension_strict,
                                        sqlite
                                            .vector_extension_path
                                            .as_deref()
                                            .unwrap_or("<unset>"),
                                        sqlite
                                            .vector_extension_entrypoint
                                            .as_deref()
                                            .unwrap_or("<default>")
                                    );
                                    if let Some(path) = sqlite.vector_extension_path.as_deref() {
                                        let exists = std::path::Path::new(path).exists();
                                        println!(
                                            "- sqlite_vector_extension_path_exists: {}",
                                            exists
                                        );
                                    }
                                } else {
                                    println!(
                                        "- sqlite_vector_extension: n/a (storage backend: {:?})",
                                        config.storage.backend
                                    );
                                }

                                let index = runtime.block_on(app.load_code_index_snapshot())?;
                                println!(
                                    "- index_snapshot: files={}, symbols={}, call_edges={}, updated_at={}",
                                    index.files.len(),
                                    index.symbols.len(),
                                    index.call_edges.len(),
                                    index.updated_at
                                );
                            }
                            cli::IndexCommand::Build => {
                                let index = runtime.block_on(app.build_code_index())?;
                                println!(
                                    "Code index built: files={}, symbols={}, dependencies={}",
                                    index.files.len(),
                                    index.symbols.len(),
                                    index.dependencies.len()
                                );
                            }
                            cli::IndexCommand::Snapshot => {
                                let index = runtime.block_on(app.load_code_index_snapshot())?;
                                println!(
                                    "Code index snapshot: files={}, symbols={}, updated_at={}",
                                    index.files.len(),
                                    index.symbols.len(),
                                    index.updated_at
                                );
                            }
                            cli::IndexCommand::Retrieve {
                                query,
                                top_k,
                                min_score,
                                path_prefix,
                                kind,
                            } => {
                                let mut filter_map = serde_json::Map::new();
                                if let Some(path_prefix) = path_prefix {
                                    filter_map.insert(
                                        "path_prefix".to_owned(),
                                        serde_json::Value::String(path_prefix),
                                    );
                                }
                                if let Some(kind) = kind {
                                    filter_map
                                        .insert("kind".to_owned(), serde_json::Value::String(kind));
                                }
                                let filters = if filter_map.is_empty() {
                                    None
                                } else {
                                    Some(serde_json::Value::Object(filter_map))
                                };

                                let response = runtime.block_on(
                                    app.retrieve_code_context(&query, top_k, min_score, filters),
                                )?;
                                println!(
                                    "Retrieved context: snippets={}, symbols={}, keyword_hits={}, vector_hits={}",
                                    response.snippets.len(),
                                    response.symbols.len(),
                                    response.keyword_hits,
                                    response.vector_hits
                                );

                                if response.snippets.is_empty() {
                                    println!("No retrieval snippets found for query '{query}'.");
                                } else {
                                    println!("Snippets:");
                                    for snippet in response.snippets {
                                        println!(
                                            "- [{}] {}:{}-{} score={:.3}",
                                            snippet.kind,
                                            snippet.file_path,
                                            snippet.line_start,
                                            snippet.line_end,
                                            snippet.score
                                        );
                                        println!("  {}", snippet.content);
                                    }
                                }
                            }
                            cli::IndexCommand::Search { query, limit } => {
                                let symbols =
                                    runtime.block_on(app.search_code_symbols(&query, limit))?;
                                if symbols.is_empty() {
                                    println!("No symbols found for query '{query}'.");
                                } else {
                                    println!("Found {} symbols:", symbols.len());
                                    for symbol in symbols {
                                        println!(
                                            "- {} [{}] {}:{}",
                                            symbol.name,
                                            symbol.symbol_type.as_str(),
                                            symbol.file_path,
                                            symbol.line
                                        );
                                    }
                                }
                            }
                        }

                        return Ok(());
                    }
                    cli::Command::Taxonomy { command } => {
                        match command {
                            cli::TaxonomyCommand::List => {
                                let baskets = app.runtime().taxonomy.list_baskets();
                                if baskets.is_empty() {
                                    println!("No taxonomy baskets configured.");
                                    return Ok(());
                                }
                                println!("Taxonomy baskets:");
                                for basket in baskets {
                                    let sub = app
                                        .runtime()
                                        .taxonomy
                                        .list_sub_baskets(&basket)
                                        .unwrap_or_default();
                                    if sub.is_empty() {
                                        println!("- {}", basket);
                                    } else {
                                        println!("- {} [{}]", basket, sub.join(", "));
                                    }
                                }
                            }
                            cli::TaxonomyCommand::Show { basket, sub_basket } => {
                                let items = if let Some(sub_basket) = sub_basket {
                                    app.runtime()
                                        .taxonomy
                                        .find_by_sub_basket(&basket, &sub_basket)
                                } else {
                                    app.runtime().taxonomy.find_by_basket(&basket)
                                };
                                if items.is_empty() {
                                    println!("No taxonomy items matched.");
                                    return Ok(());
                                }
                                println!("Taxonomy items:");
                                for item in items {
                                    let kind = match item.kind {
                                        rustic_ai_core::catalog::taxonomy::TaxonomyItemKind::Agent => {
                                            "agent"
                                        }
                                        rustic_ai_core::catalog::taxonomy::TaxonomyItemKind::Tool => {
                                            "tool"
                                        }
                                        rustic_ai_core::catalog::taxonomy::TaxonomyItemKind::Skill => {
                                            "skill"
                                        }
                                    };
                                    println!("- {} ({})", item.display_name, kind);
                                }
                            }
                            cli::TaxonomyCommand::Search { query } => {
                                let items = app.runtime().taxonomy.search(&query);
                                if items.is_empty() {
                                    println!("No taxonomy items matched search query.");
                                    return Ok(());
                                }
                                println!("Search matches:");
                                for item in items {
                                    let kind = match item.kind {
                                        rustic_ai_core::catalog::taxonomy::TaxonomyItemKind::Agent => {
                                            "agent"
                                        }
                                        rustic_ai_core::catalog::taxonomy::TaxonomyItemKind::Tool => {
                                            "tool"
                                        }
                                        rustic_ai_core::catalog::taxonomy::TaxonomyItemKind::Skill => {
                                            "skill"
                                        }
                                    };
                                    println!("- {} ({})", item.display_name, kind);
                                }
                            }
                        }
                        return Ok(());
                    }
                    cli::Command::Session { command } => {
                        handle_session_command(&app, command)?;
                        return Ok(());
                    }
                    cli::Command::Chat { agent, output } => {
                        let app = std::sync::Arc::new(app);
                        let repl = repl::Repl::new(app, agent.clone(), output, config_path.clone());
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
        rustic_ai_core::Error::Timeout(_) => "timeout_error",
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
            for required in ["model", "base_url"] {
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

            let auth_mode = provider_obj
                .get("auth_mode")
                .and_then(|value| value.as_str())
                .unwrap_or_default();

            if auth_mode == "api_key" {
                let value = provider_obj.get("api_key_env").ok_or_else(|| {
                    rustic_ai_core::Error::Validation(format!(
                        "strict validation requires providers[{index}].api_key_env for open_ai when auth_mode is api_key"
                    ))
                })?;

                if value.is_null() {
                    return Err(rustic_ai_core::Error::Validation(format!(
                        "strict validation does not allow null for providers[{index}].api_key_env"
                    )));
                }

                if value.as_str().map(|s| s.trim().is_empty()).unwrap_or(false) {
                    return Err(rustic_ai_core::Error::Validation(format!(
                        "strict validation does not allow empty values for providers[{index}].api_key_env"
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
