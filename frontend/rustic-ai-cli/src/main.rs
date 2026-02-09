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

    println!("rustic-ai-cli initialized in {:?} mode", app.config().mode);
    Ok(())
}
