use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "localasr", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon (default if no subcommand given)
    Daemon,
    /// (placeholder) GUI setup wizard — see Plan 3
    Setup,
    /// (placeholder) Diagnostics — see Plan 2
    Doctor,
    /// (placeholder) Term database extractor — see Plan 4
    Extract { folder: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Cmd::Daemon) {
        Cmd::Daemon => {
            tracing::info!("daemon: not yet implemented");
            Ok(())
        }
        Cmd::Setup => {
            anyhow::bail!("`localasr setup` lands in Plan 3");
        }
        Cmd::Doctor => {
            anyhow::bail!("`localasr doctor` lands in Plan 2");
        }
        Cmd::Extract { .. } => {
            anyhow::bail!("`localasr extract` lands in Plan 4");
        }
    }
}
