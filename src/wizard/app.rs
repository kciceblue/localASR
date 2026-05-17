//! egui app shell. Currently shows a placeholder screen with the current step
//! name; real step rendering lands in Tasks 3+.

use crate::wizard::runtime::WizardRuntime;
use crate::wizard::state::WizardState;
use eframe::egui;

pub struct WizardApp {
    pub state: WizardState,
    pub runtime: WizardRuntime,
}

impl WizardApp {
    pub fn new(runtime: WizardRuntime) -> Self {
        Self {
            state: WizardState::new(),
            runtime,
        }
    }
}

impl eframe::App for WizardApp {
    // eframe 0.34 replaced `update(ctx, frame)` with `ui(ui, frame)`: the
    // root `CentralPanel` is now provided automatically, so we render
    // directly into the supplied `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("localasr setup");
        ui.label(format!("current step: {:?}", self.state.step));
        ui.add_space(8.0);
        ui.label("(individual steps land in later tasks)");
    }
}
