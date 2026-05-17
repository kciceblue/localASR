//! `localasr setup` — interactive GUI wizard.

pub mod app;
pub mod runtime;
pub mod state;

use anyhow::Result;
use app::WizardApp;
use runtime::WizardRuntime;

pub fn run() -> Result<()> {
    let runtime = WizardRuntime::start()?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([720.0, 520.0])
            .with_title("localasr setup"),
        ..Default::default()
    };
    eframe::run_native(
        "localasr setup",
        options,
        Box::new(|_cc| Ok(Box::new(WizardApp::new(runtime)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;
    Ok(())
}
