//! ASR endpoint step: collect base URL / API key / model and let the user
//! verify them with a non-blocking Test button. The async probe runs on the
//! wizard's tokio runtime; result returns via a oneshot polled each frame.

use eframe::egui;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::config::AsrConfig;
use crate::doctor::probes::asr_probe;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct AsrStepState {
    pub pending: Option<oneshot::Receiver<Result<(), String>>>,
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut AsrStepState,
    rt_handle: &Handle,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("ASR endpoint");
    ui.add_space(8.0);

    // While a probe is in flight, egui won't naturally repaint until user
    // input arrives — force a tick so the "testing…" state actually flips
    // back to a result without the user wiggling the mouse.
    if step_state.pending.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }

    ui.label("base URL:");
    ui.text_edit_singleline(&mut state.asr_base_url);
    ui.add_space(4.0);
    ui.label("API key (use the literal key, or '${ENV_VAR}' to defer):");
    ui.text_edit_singleline(&mut state.asr_api_key);
    ui.add_space(4.0);
    ui.label("model:");
    ui.text_edit_singleline(&mut state.asr_model);

    ui.add_space(12.0);

    // Poll pending test result
    if let Some(rx) = step_state.pending.as_mut() {
        use tokio::sync::oneshot::error::TryRecvError;
        match rx.try_recv() {
            Ok(result) => {
                state.asr_test_result = Some(result);
                step_state.pending = None;
            }
            Err(TryRecvError::Empty) => {} // still in flight
            Err(TryRecvError::Closed) => {
                state.asr_test_result = Some(Err("test task crashed before reporting".into()));
                step_state.pending = None;
            }
        }
    }

    let testing = step_state.pending.is_some();
    let test_btn = ui.add_enabled(
        !testing,
        egui::Button::new(if testing { "testing…" } else { "test" }),
    );
    if test_btn.clicked() {
        let cfg = AsrConfig {
            base_url: state.asr_base_url.clone(),
            api_key: state.asr_api_key.clone(),
            model: state.asr_model.clone(),
            language: "".into(),
            timeout_ms: 10000,
        };
        let (tx, rx) = oneshot::channel();
        rt_handle.spawn(async move {
            let result = asr_probe(cfg).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        step_state.pending = Some(rx);
    }

    if let Some(r) = &state.asr_test_result {
        match r {
            Ok(()) => {
                ui.colored_label(
                    egui::Color32::from_rgb(80, 200, 120),
                    "✓ endpoint responded OK",
                );
            }
            Err(e) => {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("✗ {e}"));
            }
        }
    }

    let next_ok = state
        .asr_test_result
        .as_ref()
        .map(|r| r.is_ok())
        .unwrap_or(false);
    nav(
        ui,
        WizardStep::Mic,
        if next_ok { Some(WizardStep::Editor) } else { None },
    )
}
