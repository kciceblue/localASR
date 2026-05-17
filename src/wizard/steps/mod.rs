pub mod asr;
pub mod editor;
pub mod hotkey;
pub mod mic;
pub mod save;
pub mod test;
pub mod welcome;

use crate::wizard::state::WizardStep;
use eframe::egui;

/// Renders a horizontal "back  next" footer. `next: None` disables the next
/// button (e.g., when required fields aren't filled in yet). Returns the
/// target step if either button was clicked.
pub fn nav(ui: &mut egui::Ui, back: WizardStep, next: Option<WizardStep>) -> Option<WizardStep> {
    let mut clicked = None;
    ui.add_space(16.0);
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("back").clicked() {
            clicked = Some(back);
        }
        let next_enabled = next.is_some();
        let resp = ui.add_enabled(next_enabled, egui::Button::new("next"));
        if resp.clicked() {
            clicked = next;
        }
    });
    clicked
}
