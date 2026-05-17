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

impl WizardStep {
    pub fn next(self) -> WizardStep {
        match self {
            WizardStep::Welcome => WizardStep::Mic,
            WizardStep::Mic => WizardStep::Asr,
            WizardStep::Asr => WizardStep::Editor,
            WizardStep::Editor => WizardStep::Hotkey,
            WizardStep::Hotkey => WizardStep::Test,
            WizardStep::Test => WizardStep::Save,
            WizardStep::Save => WizardStep::Done,
            WizardStep::Done => WizardStep::Done,
        }
    }

    pub fn prev(self) -> WizardStep {
        match self {
            WizardStep::Welcome => WizardStep::Welcome,
            WizardStep::Mic => WizardStep::Welcome,
            WizardStep::Asr => WizardStep::Mic,
            WizardStep::Editor => WizardStep::Asr,
            WizardStep::Hotkey => WizardStep::Editor,
            WizardStep::Test => WizardStep::Hotkey,
            WizardStep::Save => WizardStep::Test,
            WizardStep::Done => WizardStep::Save,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_then_prev_returns_to_start() {
        let s = WizardStep::Welcome;
        assert_eq!(s.next().prev(), s);
    }

    #[test]
    fn done_is_terminal() {
        assert_eq!(WizardStep::Done.next(), WizardStep::Done);
    }

    #[test]
    fn welcome_prev_is_self() {
        assert_eq!(WizardStep::Welcome.prev(), WizardStep::Welcome);
    }
}
