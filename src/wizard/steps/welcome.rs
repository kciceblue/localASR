use crate::wizard::state::{WizardState, WizardStep};
use eframe::egui;

pub fn render(state: &mut WizardState, ui: &mut egui::Ui) {
    ui.heading("welcome to localasr");
    ui.add_space(12.0);
    ui.label("this wizard configures your push-to-talk dictation setup:");
    ui.add_space(8.0);
    ui.label("  • pick a microphone");
    ui.label("  • point at an ASR endpoint (OpenAI-compatible)");
    ui.label("  • point at a chat/completions endpoint for polishing");
    ui.label("  • record a hotkey");
    ui.label("  • run an end-to-end test");
    ui.label("  • save your config.toml");
    ui.add_space(24.0);
    if ui.button("start").clicked() {
        state.step = WizardStep::Mic;
    }
}
