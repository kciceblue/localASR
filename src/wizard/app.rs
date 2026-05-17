//! egui app shell. Dispatches on `state.step` to the matching step renderer;
//! steps not yet implemented fall through to a placeholder + nav footer.

use crate::wizard::runtime::WizardRuntime;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps;
use crate::wizard::steps::mic::MicStepState;
use eframe::egui;

pub struct WizardApp {
    pub state: WizardState,
    pub runtime: WizardRuntime,
    pub mic: MicStepState,
}

impl WizardApp {
    pub fn new(runtime: WizardRuntime) -> Self {
        Self {
            state: WizardState::new(),
            runtime,
            mic: MicStepState::default(),
        }
    }
}

impl eframe::App for WizardApp {
    // eframe 0.34's App::ui hands us a Ui already wrapped in a CentralPanel,
    // so we render straight into it. (App::update still exists but is
    // deprecated in favor of this.)
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Force repaints so the level meter animates while we're on the mic step.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));

        let nav_result = match self.state.step {
            WizardStep::Welcome => {
                steps::welcome::render(&mut self.state, ui);
                None
            }
            WizardStep::Mic => steps::mic::render(&mut self.state, &mut self.mic, ui),
            WizardStep::Done => {
                ui.heading("done");
                ui.label("close this window and run `localasr daemon`.");
                None
            }
            _ => {
                ui.heading(format!("{:?}", self.state.step));
                ui.label("(this step lands in a later task)");
                steps::nav(ui, self.state.step.prev(), Some(self.state.step.next()))
            }
        };

        if let Some(s) = nav_result {
            self.state.step = s;
            // When leaving the mic step, drop the level meter so we release
            // the cpal stream + device handle for the rest of the session.
            if self.state.step != WizardStep::Mic {
                self.mic.meter = None;
            }
        }
    }
}
