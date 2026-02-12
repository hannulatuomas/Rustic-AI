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
    Todo {
        #[command(subcommand)]
        command: TodoCommand,
    },
    Routing {
        #[command(subcommand)]
        command: RoutingCommand,
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
pub enum TodoCommand {
    List {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long, value_enum)]
        status: Option<TodoStatus>,
        #[arg(long, value_enum)]
        priority: Option<TodoPriority>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        show_metadata: bool,
    },
    Add {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_enum)]
        priority: Option<TodoPriority>,
        #[arg(long)]
        tag: Vec<String>,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long)]
        file: Vec<String>,
        #[arg(long)]
        tool: Vec<String>,
        #[arg(long)]
        routing_trace_id: Option<String>,
        #[arg(long)]
        sub_agent_output_id: Option<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, value_enum)]
        status: Option<TodoStatus>,
        #[arg(long, value_enum)]
        priority: Option<TodoPriority>,
        #[arg(long)]
        tag: Option<Vec<String>>,
        #[arg(long)]
        file: Option<Vec<String>>,
        #[arg(long)]
        tool: Option<Vec<String>>,
        #[arg(long)]
        routing_trace_id: Option<String>,
        #[arg(long)]
        sub_agent_output_id: Option<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    Complete {
        id: String,
    },
    Delete {
        id: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TodoStatus {
    Todo,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TodoPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Subcommand)]
pub enum RoutingCommand {
    Trace {
        #[arg(long)]
        session_id: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Analyze {
        task: String,
        #[arg(long)]
        context_pressure: Option<f64>,
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
