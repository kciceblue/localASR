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
        Cmd::Doctor => run_doctor().await,
        Cmd::Extract { .. } => anyhow::bail!("`localasr extract` lands in Plan 4"),
    }
}

async fn run_doctor() -> anyhow::Result<()> {
    use localasr::doctor::{run_all, Check};

    let config_path = localasr::config::default_config_path()?;
    let config_path_for_check = config_path.clone();

    // Load config once if present so the later checks can be built from it.
    let cfg_opt = if config_path.exists() {
        Some(localasr::config::load(&config_path)?)
    } else {
        None
    };

    let mut checks: Vec<Check> = Vec::new();

    checks.push(Check::new("config", move || async move {
        localasr::doctor::probes::config_probe(config_path_for_check).await
    }));

    if let Some(cfg) = cfg_opt {
        let asr_cfg = cfg.asr.clone();
        checks.push(Check::new("asr endpoint", move || async move {
            localasr::doctor::probes::asr_probe(asr_cfg).await
        }));

        let editor_cfg = cfg.editor.clone();
        checks.push(Check::new("editor endpoint", move || async move {
            localasr::doctor::probes::editor_probe(editor_cfg).await
        }));

        let device = cfg.audio.device.clone();
        checks.push(Check::new("microphone", move || async move {
            localasr::doctor::probes::mic_probe(&device).await
        }));

        // Build the platform once, share via Arc into each probe.
        #[cfg(target_os = "linux")]
        let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::linux::LinuxPlatform::new());
        #[cfg(target_os = "windows")]
        let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::windows::WindowsPlatform::new());

        let p1 = platform.clone();
        checks.push(Check::new("clipboard", move || async move {
            localasr::doctor::probes::paste_probe(p1).await
        }));

        let p2 = platform.clone();
        checks.push(Check::new("hotkey listener", move || async move {
            localasr::doctor::probes::hotkey_probe(p2).await
        }));
    } else {
        eprintln!("(config missing — skipping endpoint/audio/platform checks)");
    }

    println!("running localasr diagnostics:");
    let failures = run_all(checks).await;
    if failures == 0 {
        println!("\nall checks passed");
        Ok(())
    } else {
        anyhow::bail!("{failures} check(s) failed")
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
