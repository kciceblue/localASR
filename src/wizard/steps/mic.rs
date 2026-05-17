//! Mic step: list cpal input devices, let the user pick one, and show a live
//! level meter so they can confirm the device is actually capturing audio.

use cpal::traits::{DeviceTrait, HostTrait};
use eframe::egui;

use crate::wizard::level_meter::LevelMeter;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

/// Per-step UI state held by the app (so the `LevelMeter` survives across
/// frames; we don't want to tear down and respawn the cpal stream every frame).
#[derive(Default)]
pub struct MicStepState {
    pub devices: Vec<String>,
    pub selected: String,
    pub meter: Option<LevelMeter>,
    /// Last `start_meter` failure, if any. Latched so we don't busy-retry
    /// the cpal stream every frame on a broken device — and so we have
    /// something to surface in the UI.
    pub meter_err: Option<String>,
}

impl MicStepState {
    pub fn refresh_devices(&mut self) {
        self.devices.clear();
        let host = cpal::default_host();
        if let Ok(iter) = host.input_devices() {
            for d in iter {
                if let Ok(name) = d.name() {
                    self.devices.push(name);
                }
            }
        }
        // Rescanning is the user's explicit "try again" signal; clear any
        // latched error so the next render attempt actually retries.
        self.meter_err = None;
    }

    /// Start the level meter for the currently-selected device, replacing any
    /// prior. Dropping the old meter first releases the cpal stream + device
    /// handle so the new stream can claim it.
    pub fn start_meter(&mut self) {
        self.meter = None; // release prior device
        self.meter_err = None; // clear stale error before retry
        match LevelMeter::start(&self.selected) {
            Ok(m) => self.meter = Some(m),
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!("level meter start failed: {msg}");
                self.meter_err = Some(msg);
            }
        }
    }
}

pub fn render(
    state: &mut WizardState,
    mic: &mut MicStepState,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("pick a microphone");
    ui.add_space(8.0);

    if mic.devices.is_empty() {
        mic.refresh_devices();
    }

    if ui.button("rescan devices").clicked() {
        mic.refresh_devices();
    }

    ui.add_space(8.0);
    ui.label("device:");
    let prev_selected = mic.selected.clone();
    egui::ComboBox::from_id_salt("mic_device")
        .selected_text(if mic.selected.is_empty() {
            "(system default)"
        } else {
            mic.selected.as_str()
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut mic.selected, String::new(), "(system default)");
            for d in &mic.devices {
                ui.selectable_value(&mut mic.selected, d.clone(), d);
            }
        });

    // Only (re)start the cpal stream when the user picks a different device,
    // or when we've never tried for this device. If a prior attempt failed,
    // `meter_err` is latched and we surface it in the UI instead of busy-
    // retrying at the repaint rate.
    if mic.selected != prev_selected || (mic.meter.is_none() && mic.meter_err.is_none()) {
        mic.start_meter();
    }

    ui.add_space(12.0);
    ui.label("speak — the bar should move:");
    if let Some(err) = &mic.meter_err {
        ui.colored_label(
            egui::Color32::from_rgb(220, 80, 80),
            format!("✗ {err}"),
        );
    }
    let level = mic.meter.as_ref().map(|m| m.current()).unwrap_or(0.0);
    // Visual: a horizontal bar 0..=1.
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(400.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let filled = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * level.clamp(0.0, 1.0), rect.height()),
    );
    painter.rect_filled(filled, 2.0, egui::Color32::from_rgb(80, 200, 120));
    ui.label(format!("level: {:.3}", level));

    // Save the selected device to wizard state so we don't lose it on back/forward.
    state.mic_device = mic.selected.clone();

    nav(ui, WizardStep::Welcome, Some(WizardStep::Asr))
}
