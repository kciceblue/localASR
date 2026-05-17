//! Per-session orchestration: consumes ChunkEvents from the chunker, calls ASR
//! + light editor per chunk, runs heavy editor on close. Applies every text
//! change via the Injector.

use crate::config::{Config, EditorConfig};
use crate::daemon::asr_client::Asr;
use crate::daemon::editor_client::Editor;
use crate::daemon::injector::Injector;
use crate::daemon::state::TypedState;
use crate::daemon::vad::{ChunkEvent, EndReason};
use crate::platform::ClipboardSnapshot;
use crate::terms::TermDb;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct Pipeline {
    pub cfg: Config,
    pub asr: Arc<dyn Asr>,
    pub editor: Arc<dyn Editor>,
    pub injector: Arc<Injector>,
    pub terms: Option<Arc<TermDb>>,
}

impl Pipeline {
    /// Run one full session. Consumes events until the channel closes.
    /// Returns when the channel closes (i.e., session aborted or audio ended).
    pub async fn run(&self, mut events: mpsc::Receiver<ChunkEvent>) -> Result<()> {
        let mut state = TypedState::new();
        let mut chunks_text: Vec<String> = Vec::new();
        let saved = self.injector.save_clipboard().await?;

        while let Some(ev) = events.recv().await {
            match ev {
                ChunkEvent::Start => {}
                ChunkEvent::Samples(samples) => {
                    let raw = match self.asr.transcribe(&samples).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("ASR failed: {e}; skipping chunk");
                            continue;
                        }
                    };
                    chunks_text.push(raw);
                    let polished = if self.cfg.editor.light.enabled {
                        let n = self.cfg.editor.light.context_chunks as usize;
                        let start = chunks_text.len().saturating_sub(n);
                        let window = &chunks_text[start..];
                        match self.editor.polish_light(window).await {
                            Ok(t) => {
                                // The light pass returns the corrected version of the LAST chunk.
                                *chunks_text.last_mut().unwrap() = t.clone();
                                t
                            }
                            Err(e) => {
                                tracing::warn!("light editor failed: {e}; using raw ASR");
                                chunks_text.last().cloned().unwrap_or_default()
                            }
                        }
                    } else {
                        chunks_text.last().cloned().unwrap_or_default()
                    };
                    // Compute the new full text and apply the diff.
                    let _ = polished;
                    let new_full = chunks_text.join(" ");
                    let diff = state.update(&new_full);
                    self.injector.apply(&diff, &saved).await?;
                }
                ChunkEvent::End { reason } => {
                    if reason == EndReason::SessionClosed {
                        self.finalize(&mut state, &chunks_text, &saved).await?;
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    async fn finalize(&self, state: &mut TypedState, chunks: &[String], saved: &ClipboardSnapshot) -> Result<()> {
        if !self.cfg.editor.heavy.enabled || chunks.is_empty() { return Ok(()); }
        let utterance = chunks.join(" ");
        let terms_ref = self.terms.as_deref();
        match self.editor.polish_heavy(&utterance, terms_ref).await {
            Ok(t) => {
                let diff = state.update(&t);
                self.injector.apply(&diff, saved).await?;
            }
            Err(e) => tracing::warn!("heavy editor failed: {e}"),
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn _enforce_editor_config_lifetime(c: &EditorConfig) -> &str { &c.model }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AsrConfig, AudioConfig, EditorPassConfig, HotkeyConfig, InjectionConfig, TermsConfig, VadBackend, VadConfig};
    use crate::daemon::vad::{ChunkEvent, EndReason};
    use crate::platform::mock::MockPlatform;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeAsr { transcripts: Mutex<std::vec::IntoIter<String>> }
    impl FakeAsr {
        fn new(xs: Vec<&str>) -> Self {
            Self { transcripts: Mutex::new(xs.into_iter().map(String::from).collect::<Vec<_>>().into_iter()) }
        }
    }
    #[async_trait]
    impl Asr for FakeAsr {
        async fn transcribe(&self, _: &[i16]) -> Result<String> {
            Ok(self.transcripts.lock().unwrap().next().unwrap_or_default())
        }
    }

    struct FakeEditor;
    #[async_trait]
    impl Editor for FakeEditor {
        async fn polish_light(&self, chunks: &[String]) -> Result<String> {
            Ok(chunks.last().cloned().unwrap_or_default())
        }
        async fn polish_heavy(&self, utt: &str, _: Option<&TermDb>) -> Result<String> {
            Ok(format!("{utt}."))
        }
    }

    fn test_cfg() -> Config {
        Config {
            hotkey: HotkeyConfig { binding: "RightCtrl".into() },
            audio: AudioConfig { device: "".into(), sample_rate: 16000 },
            vad: VadConfig { backend: VadBackend::Silero, min_silence_ms: 400, max_chunk_ms: 5000 },
            asr: AsrConfig { base_url: "".into(), api_key: "".into(), model: "".into(), language: "".into(), timeout_ms: 5000 },
            editor: EditorConfig {
                base_url: "".into(), api_key: "".into(), model: "".into(),
                light_temperature: 0.0, heavy_temperature: 0.2, light_timeout_ms: 3000, heavy_timeout_ms: 10000,
                light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
                heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
            },
            terms: TermsConfig { enabled: false, path: "".into() },
            injection: InjectionConfig { mode: "clipboard_paste".into(), paste_shortcut: "Ctrl+V".into(), restore_delay_ms: 1 },
        }
    }

    #[tokio::test]
    async fn full_session_runs_light_then_heavy() {
        let platform = MockPlatform::new();
        let inj = Arc::new(Injector::new(test_cfg().injection.clone(), platform.clone()));
        let pipeline = Pipeline {
            cfg: test_cfg(),
            asr: Arc::new(FakeAsr::new(vec!["hello", "world"])),
            editor: Arc::new(FakeEditor),
            injector: inj,
            terms: None,
        };
        let (tx, rx) = mpsc::channel(16);
        tx.send(ChunkEvent::Start).await.unwrap();
        tx.send(ChunkEvent::Samples(vec![0; 1600])).await.unwrap();
        tx.send(ChunkEvent::End { reason: EndReason::SilenceTimeout }).await.unwrap();
        tx.send(ChunkEvent::Start).await.unwrap();
        tx.send(ChunkEvent::Samples(vec![0; 1600])).await.unwrap();
        tx.send(ChunkEvent::End { reason: EndReason::SessionClosed }).await.unwrap();
        drop(tx);

        pipeline.run(rx).await.unwrap();

        // Final clipboard restore must equal the original (None — MockPlatform starts empty).
        let calls = platform.calls();
        let writes: Vec<_> = calls.iter().filter_map(|c| match c {
            crate::platform::mock::Call::WriteClipboard(s) => Some(s.clone()),
            _ => None,
        }).collect();
        assert!(writes.first().unwrap().0.as_deref().unwrap_or("").contains("hello"));
        assert_eq!(writes.last().unwrap(), &ClipboardSnapshot(None));
    }
}
