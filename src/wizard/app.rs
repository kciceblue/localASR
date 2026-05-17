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
    pub asr: steps::asr::AsrStepState,
    pub editor: steps::editor::EditorStepState,
    pub hotkey: steps::hotkey::HotkeyStepState,
}

impl WizardApp {
    pub fn new(runtime: WizardRuntime) -> Self {
        Self {
            state: WizardState::new(),
            runtime,
            mic: MicStepState::default(),
            asr: steps::asr::AsrStepState::default(),
            editor: steps::editor::EditorStepState::default(),
            hotkey: steps::hotkey::HotkeyStepState::default(),
        }
    }
}

impl eframe::App for WizardApp {
    // eframe 0.34's App::ui hands us a Ui already wrapped in a CentralPanel,
    // so we render straight into it. (App::update still exists but is
    // deprecated in favor of this.)
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let nav_result = match self.state.step {
            WizardStep::Welcome => {
                steps::welcome::render(&mut self.state, ui);
                None
            }
            WizardStep::Mic => {
                // Level meter animates — request frequent repaints only while
                // we're on this step. Other steps stay idle until input.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
                steps::mic::render(&mut self.state, &mut self.mic, ui)
            }
            WizardStep::Asr => {
                steps::asr::render(&mut self.state, &mut self.asr, &self.runtime.handle, ui)
            }
            WizardStep::Editor => steps::editor::render(
                &mut self.state,
                &mut self.editor,
                &self.runtime.handle,
                ui,
            ),
            WizardStep::Hotkey => {
                // Bind ctx separately — passing `ui.ctx()` inline with `ui`
                // tries to borrow `ui` both immutably and mutably.
                let ctx = ui.ctx().clone();
                steps::hotkey::render(&mut self.state, &mut self.hotkey, &ctx, ui)
            }
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
