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
    Agents,
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    Taxonomy {
        #[command(subcommand)]
        command: TaxonomyCommand,
    },
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
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Chat {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, value_enum, default_value = "text")]
        output: OutputFormat,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthCommand {
    Methods,
    Connect {
        #[arg(long)]
        provider: String,
        #[arg(long, value_enum)]
        method: AuthMethod,
        #[arg(long, default_value_t = false)]
        no_browser: bool,
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
    List,
    Logout {
        #[arg(long)]
        provider: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AuthMethod {
    Browser,
    Headless,
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

#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    List,
    Create {
        #[arg(long)]
        agent: Option<String>,
    },
    Continue {
        id: String,
    },
    Delete {
        id: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TaxonomyCommand {
    List,
    Show {
        basket: String,
        #[arg(long)]
        sub_basket: Option<String>,
    },
    Search {
        query: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum IndexCommand {
    Status,
    Build,
    Snapshot,
    Graph {
        #[arg(long, value_enum, default_value = "summary")]
        format: GraphFormat,
    },
    Impact {
        symbol: String,
        #[arg(long, default_value_t = 3)]
        depth: usize,
        #[arg(long, value_enum, default_value = "summary")]
        format: GraphFormat,
    },
    Retrieve {
        query: String,
        #[arg(long, default_value_t = 8)]
        top_k: usize,
        #[arg(long)]
        min_score: Option<f32>,
        #[arg(long)]
        path_prefix: Option<String>,
        #[arg(long)]
        kind: Option<String>,
    },
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GraphFormat {
    Summary,
    Json,
    Dot,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
