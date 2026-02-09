mod bridge;
mod cli;
mod renderer;
mod repl;

fn main() {
    let _ = cli::Cli::parse_args();
    println!("rustic-ai-cli initialized");
}
