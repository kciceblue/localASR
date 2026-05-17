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

    let cfg = state.to_config();
    let preview =
        toml::to_string_pretty(&cfg).unwrap_or_else(|e| format!("(serialization error: {e})"));
    ui.label("config.toml preview:");
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut preview.clone())
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
        });

    ui.add_space(8.0);

    let mut auto_advance: Option<WizardStep> = None;
    if ui.button("save and finish").clicked() {
        match save_config(&cfg) {
            Ok(path) => {
                step_state.last_save_result = Some(Ok(path));
                auto_advance = Some(WizardStep::Done);
            }
            Err(e) => {
                step_state.last_save_result = Some(Err(format!("{e:#}")));
            }
        }
    }

    if let Some(r) = &step_state.last_save_result {
        match r {
            Ok(path) => {
                ui.colored_label(
                    egui::Color32::from_rgb(80, 200, 120),
                    format!("\u{2713} saved to {}", path.display()),
                );
                ui.label("close this window and run: localasr daemon");
            }
            Err(e) => {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("\u{2717} {e}"),
                );
            }
        }
    }

    // If we just saved successfully this frame, auto-advance. Otherwise allow
    // Back/Next normally; Next stays disabled until a successful save.
    if auto_advance.is_some() {
        return auto_advance;
    }
    let next_ok = step_state
        .last_save_result
        .as_ref()
        .map(|r| r.is_ok())
        .unwrap_or(false);
    nav(
        ui,
        WizardStep::Test,
        if next_ok { Some(WizardStep::Done) } else { None },
    )
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
