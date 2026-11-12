use clap::Parser;

/// Armonika, a self-hosted music server.
#[derive(Parser)]
#[command(name = "armonika", version = armonika_core::VERSION)]
struct Cli {}

fn main() {
    Cli::parse();
}
