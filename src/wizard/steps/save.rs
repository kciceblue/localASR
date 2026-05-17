//! Save step: previews the TOML the wizard is about to write, then writes it
//! to the platform-default config path. Overwrites silently (per plan, the
//! overwrite-warning is deferred to a later iteration).

use anyhow::Result;
use eframe::egui;
use std::path::PathBuf;

use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct SaveStepState {
    /// Outcome of the last "save and finish" click — `Ok(path)` on success,
    /// `Err(msg)` on failure. `None` before the user has clicked.
    pub last_save_result: Option<Result<PathBuf, String>>,
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut SaveStepState,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("save");
    ui.add_space(8.0);
    ui.label("review the config below, then write it to disk.");
    ui.add_space(8.0);

    let cfg = state.to_config();
    let preview =
        toml::to_string_pretty(&cfg).unwrap_or_else(|e| format!("(serialization error: {e})"));

    // Scrollable monospace preview. TextEdit::multiline needs &mut String;
    // we clone into a throwaway so user edits in the widget don't escape.
    egui::ScrollArea::vertical()
        .max_height(280.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut preview.clone())
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(12)
                    .desired_width(f32::INFINITY),
            );
        });

    ui.add_space(8.0);

    if ui.button("save and finish").clicked() {
        step_state.last_save_result = Some(save_config(&cfg).map_err(|e| format!("{e:#}")));
    }

    ui.add_space(8.0);
    match &step_state.last_save_result {
        Some(Ok(path)) => {
            ui.colored_label(
                egui::Color32::from_rgb(0, 160, 0),
                format!("\u{2713} saved to {}", path.display()),
            );
            ui.label("now run `localasr daemon` to start the hotkey-driven dictation loop.");
        }
        Some(Err(e)) => {
            ui.colored_label(egui::Color32::from_rgb(200, 0, 0), format!("\u{2717} {e}"));
        }
        None => {}
    }

    let next = if matches!(step_state.last_save_result, Some(Ok(_))) {
        Some(WizardStep::Done)
    } else {
        None
    };
    nav(ui, WizardStep::Test, next)
}

fn save_config(cfg: &crate::config::Config) -> Result<PathBuf> {
    let path = crate::config::default_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, text)?;
    Ok(path)
}
