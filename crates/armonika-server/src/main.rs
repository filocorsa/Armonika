use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

use armonika_server::cli::{self, user::UserCommand};
use armonika_server::config::Config;
use armonika_server::serve;

/// Armonika, a self-hosted music server.
#[derive(Parser)]
#[command(name = "armonika", version = armonika_core::VERSION)]
struct Cli {
    /// Path to the configuration file (default: ARMONIKA_CONFIG or armonika.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server.
    Serve,
    /// Manage users.
    #[command(subcommand)]
    User(UserCommand),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(());
    };

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::load(cli.config.as_deref())?;
    match command {
        Command::Serve => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(serve::run(config)),
        Command::User(command) => {
            let (db, secrets) = cli::open_store(&config)?;
            cli::user::run(&db, &secrets, command)
        }
    }
}
