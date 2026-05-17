//! End-to-end speech test. Captures audio for a fixed window, runs ASR + the
//! heavy editor pass, displays the transcript in-window.

use eframe::egui;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::config::{AsrConfig, EditorConfig};
use crate::daemon::asr_client::{Asr, OpenAiAsr};
use crate::daemon::audio::AudioCapture;
use crate::daemon::editor_client::{Editor, OpenAiEditor};
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct TestStepState {
    pub pending: Option<oneshot::Receiver<Result<String, String>>>,
}

const RECORD_SECONDS: u64 = 5;

pub fn render(
    state: &mut WizardState,
    step_state: &mut TestStepState,
    rt_handle: &Handle,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("end-to-end test");
    ui.add_space(8.0);
    ui.label(format!(
        "click record, speak for {RECORD_SECONDS} seconds, then we'll show what was transcribed."
    ));

    // While a recording+probe is in flight, egui won't naturally repaint until
    // user input arrives — force a tick so the "recording…" state actually
    // flips back to a result without the user wiggling the mouse.
    if step_state.pending.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }

    // Poll pending result. Use the same Closed-channel pattern as
    // asr.rs/editor.rs so a panicked task surfaces as an error instead of
    // hanging the UI forever.
    if let Some(rx) = step_state.pending.as_mut() {
        use tokio::sync::oneshot::error::TryRecvError;
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(t) => state.test_transcript = Some(t),
                    Err(e) => state.test_transcript = Some(format!("(error: {e})")),
                }
                step_state.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Closed) => {
                state.test_transcript =
                    Some("(error: test task crashed before reporting)".into());
                step_state.pending = None;
            }
        }
    }

    let recording = step_state.pending.is_some();
    let label = if recording { "recording…" } else { "record" };
    if ui
        .add_enabled(!recording, egui::Button::new(label))
        .clicked()
    {
        let device = state.mic_device.clone();
        // Generous 30s ASR timeout — the test captures up to RECORD_SECONDS of
        // audio, and the upload+inference can stretch on a slow network.
        let asr_cfg = state.to_asr_config(30000);
        // Heavy pass gets 20s here (vs 15s in the editor probe) to cover the
        // larger payload coming out of a full 5s utterance.
        let editor_cfg = state.to_editor_config(5000, 20000);
        let (tx, rx) = oneshot::channel();
        rt_handle.spawn(async move {
            let result = run_test(device, asr_cfg, editor_cfg)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        step_state.pending = Some(rx);
    }

    ui.add_space(12.0);
    if let Some(t) = &state.test_transcript {
        ui.label("transcript:");
        // Clone the value into a throwaway buffer — TextEdit::multiline needs
        // &mut String, and we don't want the widget to mutate the stored
        // transcript.
        ui.add(
            egui::TextEdit::multiline(&mut t.clone())
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
    }

    let has_transcript = state.test_transcript.is_some();
    let passed = state
        .test_transcript
        .as_ref()
        .map(|t| !t.starts_with("(error") && !t.starts_with("(silence"))
        .unwrap_or(false);

    if has_transcript && !passed {
        ui.add_space(8.0);
        // Spec: "Wizard refuses to save unless user explicitly overrides
        // ('save anyway')". Surface that override only when verification
        // failed (error or silence) — never when there's no attempt yet.
        if ui.button("save anyway (skip verification)").clicked() {
            return Some(WizardStep::Save);
        }
    }

    nav(
        ui,
        WizardStep::Hotkey,
        if passed { Some(WizardStep::Save) } else { None },
    )
}

async fn run_test(
    device: String,
    asr: AsrConfig,
    editor: EditorConfig,
) -> anyhow::Result<String> {
    // Capture for RECORD_SECONDS, accumulating all frames.
    let cap = AudioCapture::start(&device, 16000)?;
    let frames_rx = cap.frames.clone();
    let collect_handle = tokio::task::spawn_blocking(move || -> Vec<i16> {
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(RECORD_SECONDS);
        while std::time::Instant::now() < deadline {
            match frames_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(buf) => all.extend(buf),
                Err(_) => {}
            }
        }
        all
    });
    let samples = collect_handle.await?;
    cap.stop();

    if samples.is_empty() {
        anyhow::bail!("no audio captured (device silent?)");
    }

    let asr_client = OpenAiAsr::new(asr)?;
    let raw = asr_client.transcribe(&samples).await?;
    if raw.trim().is_empty() {
        return Ok("(silence — no speech detected, try speaking louder)".into());
    }

    let editor_client = OpenAiEditor::new(editor)?;
    let polished = editor_client.polish_heavy(&raw, None).await?;
    Ok(polished)
}
