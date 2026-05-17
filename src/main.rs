use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "localasr", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    Daemon,
    Setup,
    Doctor,
    Extract { folder: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Cmd::Daemon) {
        Cmd::Daemon => run_daemon().await,
        Cmd::Setup => anyhow::bail!("`localasr setup` lands in Plan 3"),
        Cmd::Doctor => anyhow::bail!("`localasr doctor` lands in Plan 2"),
        Cmd::Extract { .. } => anyhow::bail!("`localasr extract` lands in Plan 4"),
    }
}

async fn run_daemon() -> anyhow::Result<()> {
    let path = localasr::config::default_config_path()?;
    if !path.exists() {
        anyhow::bail!(
            "No config found at {}. Run: localasr setup",
            path.display()
        );
    }
    let cfg = localasr::config::load(&path)?;

    #[cfg(target_os = "linux")]
    let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::linux::LinuxPlatform::new());
    #[cfg(target_os = "windows")]
    let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::windows::WindowsPlatform::new());

    localasr::daemon::run(cfg, platform).await
}
