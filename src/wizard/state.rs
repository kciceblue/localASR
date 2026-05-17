//! Wizard state machine and collected values. Steps mutate this; on save it
//! serializes to a `Config`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Welcome,
    Mic,
    Asr,
    Editor,
    Hotkey,
    Test,
    Save,
    Done,
}

#[derive(Debug, Default, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    // Fields land in later tasks as their steps need them.
}

impl Default for WizardStep {
    fn default() -> Self {
        WizardStep::Welcome
    }
}

impl WizardState {
    pub fn new() -> Self {
        Self::default()
    }
}
