//! Wizard state machine and collected values. Steps mutate this; on save it
//! serializes to a `Config`.

use crate::config::{
    AsrConfig, AudioConfig, Config, EditorConfig, EditorPassConfig, HotkeyConfig, InjectionConfig,
    TermsConfig, VadBackend, VadConfig,
};

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

    /// Build an AsrConfig for probing or one-shot use.
    /// `timeout_ms` is the only thing that varies per call site.
    pub fn to_asr_config(&self, timeout_ms: u64) -> AsrConfig {
        AsrConfig {
            base_url: self.asr_base_url.clone(),
            api_key: self.asr_api_key.clone(),
            model: self.asr_model.clone(),
            language: String::new(),
            timeout_ms,
        }
    }

    /// Build an EditorConfig for probing or one-shot use.
    /// `light_timeout_ms` / `heavy_timeout_ms` vary per call site.
    pub fn to_editor_config(&self, light_timeout_ms: u64, heavy_timeout_ms: u64) -> EditorConfig {
        EditorConfig {
            base_url: self.editor_base_url.clone(),
            api_key: self.editor_api_key.clone(),
            model: self.editor_model.clone(),
            light_temperature: 0.0,
            heavy_temperature: 0.2,
            light_timeout_ms,
            heavy_timeout_ms,
            light: EditorPassConfig {
                enabled: true,
                context_chunks: 3,
                system_prompt: String::new(),
            },
            heavy: EditorPassConfig {
                enabled: true,
                context_chunks: 0,
                system_prompt: String::new(),
            },
        }
    }

    /// Build a complete Config from the values the wizard collected.
    /// Fields not surfaced in the UI get sensible defaults aligned with the
    /// daemon's documented defaults.
    pub fn to_config(&self) -> Config {
        Config {
            hotkey: HotkeyConfig {
                binding: self.hotkey_binding.clone(),
            },
            audio: AudioConfig {
                device: self.mic_device.clone(),
                sample_rate: 16000,
            },
            vad: VadConfig {
                backend: VadBackend::Silero,
                min_silence_ms: 400,
                max_chunk_ms: 5000,
            },
            asr: self.to_asr_config(15000),
            editor: self.to_editor_config(4000, 15000),
            terms: TermsConfig {
                enabled: false,
                path: "~/.config/localasr/terms.toml".into(),
            },
            injection: InjectionConfig {
                mode: "clipboard_paste".into(),
                paste_shortcut: "Ctrl+V".into(),
                restore_delay_ms: 100,
            },
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

#[cfg(test)]
mod cfg_tests {
    use super::*;

    #[test]
    fn to_config_round_trips_through_toml() {
        let mut s = WizardState::new();
        s.asr_api_key = "sk-test".into();
        s.editor_api_key = "sk-test".into();
        s.hotkey_binding = "F8".into();
        let cfg = s.to_config();
        let serialized = toml::to_string(&cfg).unwrap();
        let parsed: crate::config::Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.hotkey.binding, "F8");
        assert_eq!(parsed.asr.model, "whisper-1");
        assert_eq!(parsed.editor.model, "gpt-4o-mini");
        assert_eq!(parsed.vad.backend, crate::config::VadBackend::Silero);
        assert_eq!(parsed.injection.mode, "clipboard_paste");
        assert_eq!(parsed.vad.min_silence_ms, 400);
        assert_eq!(parsed.editor.light.enabled, true);
    }
}
