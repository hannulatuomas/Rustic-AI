use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "rustic-ai", about = "Rustic-AI CLI")]
pub struct Cli {
    #[arg(long, default_value = "config.toml")]
    pub config: String,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
