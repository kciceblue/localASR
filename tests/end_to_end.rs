//! Drives the pipeline directly with mocks for ASR, editor, and platform.
//! Exercises the same code paths the daemon takes, minus real hotkey + audio.

use localasr::config::*;
use localasr::daemon::asr_client::Asr;
use localasr::daemon::editor_client::Editor;
use localasr::daemon::injector::Injector;
use localasr::daemon::pipeline::Pipeline;
use localasr::daemon::vad::{ChunkEvent, EndReason};
use localasr::platform::mock::{Call, MockPlatform};
use localasr::platform::ClipboardSnapshot;
use localasr::terms::TermDb;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct ScriptedAsr(Mutex<std::vec::IntoIter<String>>);
#[async_trait]
impl Asr for ScriptedAsr {
    async fn transcribe(&self, _: &[i16]) -> anyhow::Result<String> {
        Ok(self.0.lock().unwrap().next().unwrap_or_default())
    }
}

struct ScriptedEditor;
#[async_trait]
impl Editor for ScriptedEditor {
    async fn polish_light(&self, chunks: &[String]) -> anyhow::Result<String> {
        Ok(chunks.last().cloned().unwrap_or_default())
    }
    async fn polish_heavy(&self, utt: &str, _: Option<&TermDb>) -> anyhow::Result<String> {
        Ok(utt.trim().to_string() + ".")
    }
}

fn cfg() -> Config {
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
async fn two_chunks_then_heavy_pass_produces_correct_call_sequence() {
    let platform = MockPlatform::new();
    let inj = Arc::new(Injector::new(cfg().injection.clone(), platform.clone()));
    let pipeline = Pipeline {
        cfg: cfg(),
        asr: Arc::new(ScriptedAsr(Mutex::new(vec!["hello".to_string(), "world".to_string()].into_iter()))),
        editor: Arc::new(ScriptedEditor),
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

    let calls = platform.calls();
    // First call is always: read clipboard for save.
    assert_eq!(calls[0], Call::ReadClipboard);
    // Last clipboard-write must restore to the saved value (None for a fresh mock).
    let last_write = calls.iter().rev().find_map(|c| match c {
        Call::WriteClipboard(s) => Some(s.clone()),
        _ => None,
    }).unwrap();
    assert_eq!(last_write, ClipboardSnapshot(None));
    // Heavy pass should have produced a paste with trailing period.
    let heavy_paste = calls.iter().rev().find_map(|c| match c {
        Call::WriteClipboard(ClipboardSnapshot(Some(t))) if t.ends_with('.') => Some(t.clone()),
        _ => None,
    });
    assert!(heavy_paste.is_some(), "heavy editor's pasted text should end with a period");
}
