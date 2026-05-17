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
    /// Selected input device name; empty string = system default. Populated by
    /// the Mic step and persisted across back/forward navigation.
    pub mic_device: String,

    // ASR endpoint
    pub asr_base_url: String,
    pub asr_api_key: String,
    pub asr_model: String,
    pub asr_test_result: Option<Result<(), String>>,

    // Editor endpoint
    pub editor_base_url: String,
    pub editor_api_key: String,
    pub editor_model: String,
    pub editor_test_result: Option<Result<(), String>>,

    // Hotkey
    pub hotkey_binding: String,

    // Test step
    pub test_transcript: Option<String>,
}

impl Default for WizardStep {
    fn default() -> Self {
        WizardStep::Welcome
    }
}

impl WizardState {
    pub fn new() -> Self {
        Self {
            asr_base_url: "https://api.openai.com/v1".into(),
            asr_model: "whisper-1".into(),
            editor_base_url: "https://api.openai.com/v1".into(),
            editor_model: "gpt-4o-mini".into(),
            hotkey_binding: "RightCtrl".into(),
            ..Default::default()
        }
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
    fn next_then_prev_is_identity_on_non_terminal_steps() {
        for s in [
            WizardStep::Welcome,
            WizardStep::Mic,
            WizardStep::Asr,
            WizardStep::Editor,
            WizardStep::Hotkey,
            WizardStep::Test,
            WizardStep::Save,
        ] {
            let n = s.next();
            // Done is terminal; everything before it should roundtrip.
            assert_eq!(n.prev(), s, "round-trip failed at {s:?}");
        }
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
