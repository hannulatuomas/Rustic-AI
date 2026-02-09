mod bridge;
mod cli;
mod renderer;
mod repl;

fn main() {
    if let Err(error) = run() {
        eprintln!("rustic-ai-cli failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> rustic_ai_core::Result<()> {
    let args = cli::Cli::parse_args();
    let config_path = std::path::PathBuf::from(args.config);
    let app = rustic_ai_core::RusticAI::from_config_path(&config_path)?;

    if let Some(command) = args.command {
        match command {
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
                let session_id = if let Some(value) = args.session_id {
                    uuid::Uuid::parse_str(&value).map_err(|err| {
                        rustic_ai_core::Error::Config(format!(
                            "invalid --session-id '{value}': {err}"
                        ))
                    })?
                } else {
                    tokio::runtime::Runtime::new()
                        .map_err(|err| {
                            rustic_ai_core::Error::Config(format!(
                                "failed to create tokio runtime: {err}"
                            ))
                        })?
                        .block_on(app.session_manager().create_session("default"))?
                };

                let topics = tokio::runtime::Runtime::new()
                    .map_err(|err| {
                        rustic_ai_core::Error::Config(format!(
                            "failed to create tokio runtime: {err}"
                        ))
                    })?
                    .block_on(app.session_manager().get_session_topics(session_id))?
                    .unwrap_or_default();
                println!("Session: {session_id}");
                println!("Topics: {}", topics.join(", "));
                return Ok(());
            }
        }
    }

    println!("rustic-ai-cli initialized in {:?} mode", app.config().mode);
    Ok(())
}
