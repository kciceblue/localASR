//! Hotkey-capture step. We use egui's own input events (focused-window key
//! events, not global) to record what the user pressed. The result is a
//! binding string compatible with the Linux/Windows hotkey parsers.

use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;
use eframe::egui;

#[derive(Default)]
pub struct HotkeyStepState {
    pub capturing: bool,
}

/// Map egui Key → the string our binding parser accepts. Returns None for keys
/// we don't support (e.g. arrow keys, letters/digits — those the user can type
/// directly into the text field).
fn key_to_binding_name(k: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match k {
        Space => "Space",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        _ => return None,
    })
}

/// Build a binding string from the current egui modifier state + a non-modifier key.
fn build_chord(mods: egui::Modifiers, key: Option<&'static str>) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.shift {
        parts.push("Shift");
    }
    if mods.alt {
        parts.push("Alt");
    }
    // `command` covers ⌘ on macOS and Ctrl-as-command elsewhere; `mac_cmd` is
    // mac-only. Either signals the Meta/Super key as far as our parser cares.
    if mods.mac_cmd || mods.command {
        parts.push("Meta");
    }
    if let Some(k) = key {
        parts.push(k);
    }
    parts.join("+")
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut HotkeyStepState,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    // While capturing, egui won't repaint on its own between key events
    // (no mouse motion to drive a redraw). Force a continuous tick so the
    // first keypress is processed promptly.
    if step_state.capturing {
        ui.ctx().request_repaint();
    }

    ui.heading("hotkey");
    ui.add_space(8.0);
    ui.label("press the key (or chord) you want to hold for push-to-talk.");
    ui.label("supported keys: F1–F12, Space, plus modifiers Ctrl/Shift/Alt/Meta.");
    ui.label("the daemon also accepts single modifier keys (RightCtrl is the default).");
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("current binding:");
        ui.monospace(&state.hotkey_binding);
    });

    ui.add_space(8.0);
    if step_state.capturing {
        ui.colored_label(
            egui::Color32::from_rgb(220, 180, 80),
            "listening… press a key",
        );
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                {
                    if let Some(name) = key_to_binding_name(*key) {
                        state.hotkey_binding = build_chord(*modifiers, Some(name));
                        step_state.capturing = false;
                        break;
                    }
                }
            }
        });
    } else if ui.button("record new binding").clicked() {
        step_state.capturing = true;
    }

    ui.add_space(8.0);
    ui.label("(or type it directly — the daemon's parser accepts the same syntax)");
    ui.text_edit_singleline(&mut state.hotkey_binding);

    nav(ui, WizardStep::Editor, Some(WizardStep::Test))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_only() {
        let mods = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(build_chord(mods, None), "Ctrl");
    }

    #[test]
    fn chord_with_key() {
        let mods = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(build_chord(mods, Some("Space")), "Ctrl+Alt+Space");
    }

    #[test]
    fn key_only() {
        assert_eq!(
            build_chord(egui::Modifiers::default(), Some("F8")),
            "F8"
        );
    }
}
