use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "rustic-ai", about = "Rustic-AI CLI")]
pub struct Cli {
    #[arg(long, default_value = "config.json")]
    pub config: String,

    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(long)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    Discover,
    Topics,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
