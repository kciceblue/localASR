//! egui app shell. Dispatches on `state.step` to the matching step renderer;
//! steps not yet implemented fall through to a placeholder + nav footer.

use crate::wizard::runtime::WizardRuntime;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps;
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
        match self.state.step {
            WizardStep::Welcome => steps::welcome::render(&mut self.state, ui),
            WizardStep::Done => {
                ui.heading("done");
                ui.label("close this window and run `localasr daemon`.");
            }
            _ => {
                ui.heading(format!("{:?}", self.state.step));
                ui.label("(this step lands in a later task)");
                if let Some(s) =
                    steps::nav(ui, self.state.step.prev(), Some(self.state.step.next()))
                {
                    self.state.step = s;
                }
            }
        }
    }
}
