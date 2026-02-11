use crate::cli::OutputFormat;
use rustic_ai_core::events::Event;
use rustic_ai_core::permissions::AskResolution;
use std::io::Write;

pub struct Renderer {
    output_format: OutputFormat,
}

impl Renderer {
    pub fn new(output_format: OutputFormat) -> Self {
        Self { output_format }
    }

    pub fn render_event(&self, event: &Event) {
        match self.output_format {
            OutputFormat::Text => self.render_text(event),
            OutputFormat::Json => self.render_json(event),
        }
    }

    fn render_text(&self, event: &Event) {
        match event {
            Event::Progress(msg) => println!("[progress] {msg}"),
            Event::ModelChunk { text, .. } => {
                print!("{text}");
                std::io::stdout().flush().ok();
            }
            Event::AgentThinking { agent, .. } => {
                println!();
                println!("[agent:{agent}] Thinking...");
            }
            Event::ToolStarted { args, .. } => {
                println!();
                println!(
                    "[tool] Executing: {}",
                    serde_json::to_string(args).unwrap_or_else(|_| "<invalid args>".to_string())
                );
            }
            Event::ToolOutput {
                tool: _,
                stdout_chunk,
                stderr_chunk,
            } => {
                if !stdout_chunk.is_empty() {
                    print!("{stdout_chunk}");
                    std::io::stdout().flush().ok();
                }
                if !stderr_chunk.is_empty() {
                    eprint!("{stderr_chunk}");
                    std::io::stderr().flush().ok();
                }
            }
            Event::ToolCompleted { tool, exit_code } => {
                let status = if *exit_code == 0 { "OK" } else { "FAILED" };
                println!();
                println!("[tool:{tool}] {status} (exit {exit_code})");
            }
            Event::WorkflowStarted {
                workflow,
                entrypoint,
                recursion_depth,
            } => {
                println!();
                println!(
                    "[workflow:{workflow}] started (entrypoint={entrypoint}, depth={recursion_depth})"
                );
            }
            Event::WorkflowStepStarted {
                workflow,
                step_id,
                step_name,
                kind,
            } => {
                println!("[workflow:{workflow}] step {step_id} ({step_name}) started [{kind}]");
            }
            Event::WorkflowStepCompleted {
                workflow,
                step_id,
                success,
                output_count,
            } => {
                let status = if *success { "OK" } else { "FAILED" };
                println!(
                    "[workflow:{workflow}] step {step_id} {status} (mapped outputs: {output_count})"
                );
            }
            Event::WorkflowCompleted {
                workflow,
                success,
                steps_executed,
            } => {
                let status = if *success { "OK" } else { "FAILED" };
                println!("[workflow:{workflow}] {status} (steps executed: {steps_executed})");
            }
            Event::PermissionRequest { tool, args, .. } => {
                println!();
                println!(
                    "[permission] Allow tool '{tool}' with args: {}? (y/n/a/d)",
                    serde_json::to_string(args).unwrap_or_else(|_| "<invalid args>".to_string())
                );
                println!("  y = allow once");
                println!("  n = deny");
                println!("  a = allow in session");
            }
            Event::PermissionDecision { tool, decision, .. } => {
                let desc = match decision {
                    AskResolution::AllowOnce => "allowed once",
                    AskResolution::AllowInSession => "allowed in session",
                    AskResolution::Deny => "denied",
                };
                println!("[permission] Tool '{tool}' {desc}");
            }
            Event::SudoSecretPrompt {
                command, reason, ..
            } => {
                println!();
                println!("[sudo] {reason}: {command}");
                println!("[sudo] Waiting for secure password input.");
            }
            Event::SubAgentCallStarted {
                caller_agent,
                target_agent,
                max_context_messages,
                ..
            } => {
                println!(
                    "[sub-agent] {caller_agent} -> {target_agent} (context messages: {max_context_messages})"
                );
            }
            Event::SubAgentCallCompleted {
                caller_agent,
                target_agent,
                success,
                ..
            } => {
                let status = if *success { "OK" } else { "FAILED" };
                println!("[sub-agent] {caller_agent} <- {target_agent} {status}");
            }
            Event::SessionUpdated(_) => {
                // Silent for now, useful for debugging
            }
            Event::Error(err) => {
                eprintln!();
                eprintln!("[error] {err}");
            }
        }
    }

    fn render_json(&self, event: &Event) {
        let output = match event {
            Event::Progress(msg) => serde_json::json!({
                "type": "progress",
                "message": msg
            }),
            Event::ModelChunk {
                session_id,
                agent,
                text,
            } => serde_json::json!({
                "type": "model_chunk",
                "session_id": session_id,
                "agent": agent,
                "text": text
            }),
            Event::AgentThinking { session_id, agent } => serde_json::json!({
                "type": "agent_thinking",
                "session_id": session_id,
                "agent": agent
            }),
            Event::ToolStarted { tool, args } => serde_json::json!({
                "type": "tool_started",
                "tool": tool,
                "args": args
            }),
            Event::ToolOutput {
                tool,
                stdout_chunk,
                stderr_chunk,
            } => serde_json::json!({
                "type": "tool_output",
                "tool": tool,
                "stdout": stdout_chunk,
                "stderr": stderr_chunk
            }),
            Event::ToolCompleted { tool, exit_code } => serde_json::json!({
                "type": "tool_completed",
                "tool": tool,
                "exit_code": exit_code
            }),
            Event::WorkflowStarted {
                workflow,
                entrypoint,
                recursion_depth,
            } => serde_json::json!({
                "type": "workflow_started",
                "workflow": workflow,
                "entrypoint": entrypoint,
                "recursion_depth": recursion_depth,
            }),
            Event::WorkflowStepStarted {
                workflow,
                step_id,
                step_name,
                kind,
            } => serde_json::json!({
                "type": "workflow_step_started",
                "workflow": workflow,
                "step_id": step_id,
                "step_name": step_name,
                "kind": kind,
            }),
            Event::WorkflowStepCompleted {
                workflow,
                step_id,
                success,
                output_count,
            } => serde_json::json!({
                "type": "workflow_step_completed",
                "workflow": workflow,
                "step_id": step_id,
                "success": success,
                "output_count": output_count,
            }),
            Event::WorkflowCompleted {
                workflow,
                success,
                steps_executed,
            } => serde_json::json!({
                "type": "workflow_completed",
                "workflow": workflow,
                "success": success,
                "steps_executed": steps_executed,
            }),
            Event::PermissionRequest {
                session_id,
                tool,
                args,
            } => serde_json::json!({
                "type": "permission_request",
                "session_id": session_id,
                "tool": tool,
                "args": args
            }),
            Event::PermissionDecision {
                session_id,
                tool,
                decision,
            } => serde_json::json!({
                "type": "permission_decision",
                "session_id": session_id,
                "tool": tool,
                "decision": decision
            }),
            Event::SudoSecretPrompt {
                session_id,
                tool,
                args,
                command,
                reason,
            } => serde_json::json!({
                "type": "sudo_secret_prompt",
                "session_id": session_id,
                "tool": tool,
                "args": args,
                "command": command,
                "reason": reason
            }),
            Event::SubAgentCallStarted {
                session_id,
                caller_agent,
                target_agent,
                max_context_messages,
            } => serde_json::json!({
                "type": "sub_agent_call_started",
                "session_id": session_id,
                "caller_agent": caller_agent,
                "target_agent": target_agent,
                "max_context_messages": max_context_messages,
            }),
            Event::SubAgentCallCompleted {
                session_id,
                caller_agent,
                target_agent,
                success,
            } => serde_json::json!({
                "type": "sub_agent_call_completed",
                "session_id": session_id,
                "caller_agent": caller_agent,
                "target_agent": target_agent,
                "success": success,
            }),
            Event::SessionUpdated(id) => serde_json::json!({
                "type": "session_updated",
                "session_id": id
            }),
            Event::Error(err) => serde_json::json!({
                "type": "error",
                "message": err
            }),
        };

        println!("{}", serde_json::to_string(&output).unwrap_or_default());
    }
}
