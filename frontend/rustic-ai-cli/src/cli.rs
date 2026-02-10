use clap::{Parser, Subcommand, ValueEnum};

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
    ValidateConfig {
        #[arg(long, default_value = "docs/config.schema.json")]
        schema: String,
        #[arg(long, default_value_t = false)]
        strict: bool,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
        #[arg(long, value_enum, default_value = "text")]
        output: OutputFormat,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    Snapshot,
    Get {
        #[arg(long)]
        path: String,
        #[arg(long, value_enum, default_value = "effective")]
        scope: ConfigReadScope,
        #[arg(long, default_value_t = false)]
        explain: bool,
    },
    Set {
        #[arg(long, value_enum)]
        scope: ConfigWriteScope,
        #[arg(long)]
        path: String,
        #[arg(long)]
        value_json: String,
        #[arg(long)]
        expected_version: Option<u64>,
    },
    Unset {
        #[arg(long, value_enum)]
        scope: ConfigWriteScope,
        #[arg(long)]
        path: String,
        #[arg(long)]
        expected_version: Option<u64>,
    },
    Patch {
        #[arg(long)]
        file: String,
        #[arg(long)]
        expected_version: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigReadScope {
    Effective,
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigWriteScope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
