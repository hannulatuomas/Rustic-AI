use crate::cli::OutputFormat;
use crate::renderer::Renderer;
use rustic_ai_core::error::Result;
use rustic_ai_core::events::Event;
use rustic_ai_core::RusticAI;
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct Repl {
    app: Arc<RusticAI>,
    agent_name: Option<String>,
    output_format: OutputFormat,
}

impl Repl {
    pub fn new(
        app: Arc<RusticAI>,
        agent_name: Option<String>,
        output_format: OutputFormat,
    ) -> Self {
        Self {
            app,
            agent_name,
            output_format,
        }
    }

    pub async fn run(&self) -> Result<()> {
        // Get or create session
        let runtime = tokio::runtime::Runtime::new().map_err(|err| {
            rustic_ai_core::Error::Config(format!("failed to create tokio runtime: {err}"))
        })?;

        let session_id = runtime.block_on(async {
            // Try to find existing session or create new one
            let sessions = self.app.session_manager().list_sessions(None).await?;
            if sessions.is_empty() {
                self.app.session_manager().create_session("default").await
            } else {
                // Use most recent session
                Ok(sessions[0].id)
            }
        })?;

        println!("Session: {session_id}");

        // Create channel for agent events
        let (event_tx, event_rx) = mpsc::channel(100);

        // Start renderer task
        let renderer = Renderer::new(self.output_format);
        let renderer_handle = tokio::spawn(async move {
            renderer.run(event_rx).await;
        });

        // Get agent
        let agent_name = self
            .agent_name
            .clone()
            .unwrap_or_else(|| "default".to_string());
        if !self.app.runtime().agents.has_agent(&agent_name) {
            return Err(rustic_ai_core::Error::NotFound(format!(
                "agent '{}' not found",
                agent_name
            )));
        }

        println!();
        println!("Rustic-AI Interactive Chat");
        println!("Type 'exit' or press Ctrl-C to quit");
        println!();

        // Main REPL loop
        loop {
            // Read user input
            print!("> ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .map_err(rustic_ai_core::Error::Io)?;

            let input = input.trim();

            // Check for exit command
            if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
                println!("Goodbye!");
                break;
            }

            // Skip empty input
            if input.is_empty() {
                continue;
            }

            // Handle permission resolution (if pending)
            if input.starts_with('y') || input.starts_with('n') || input.starts_with('a') {
                // Parse user decision
                let _decision = if input.starts_with('y') {
                    rustic_ai_core::permissions::AskResolution::AllowOnce
                } else if input.starts_with('n') {
                    rustic_ai_core::permissions::AskResolution::Deny
                } else if input.starts_with('a') {
                    rustic_ai_core::permissions::AskResolution::AllowInSession
                } else {
                    continue;
                };

                // TODO: Resolve pending permission request
                // This requires tracking the current pending permission state
                println!("(Permission resolution pending - TODO)");
                continue;
            }

            // Get agent and execute turn
            let agent = self.app.runtime().agents.get_agent(Some(&agent_name))?;

            // Spawn agent turn in background
            let agent_clone = agent.clone();
            let session_id_clone = session_id;
            let input_clone = input.to_string();
            let event_tx_clone = event_tx.clone();
            let event_tx_error = event_tx.clone();

            tokio::spawn(async move {
                if let Err(err) = agent_clone
                    .start_turn(session_id_clone, input_clone, event_tx_clone)
                    .await
                {
                    let _ = event_tx_error.try_send(Event::Error(err.to_string()));
                }
            });

            // Wait for turn to complete (simplified for now)
            // In production, we'd wait for a SessionUpdated event
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Wait for renderer to finish
        renderer_handle.await.ok();
        Ok(())
    }
}
