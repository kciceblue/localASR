use crate::platform::HotkeyEvent;
use anyhow::{Context, Result};
use evdev::{Device, EventSummary, KeyCode};
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;
use tokio::sync::mpsc;

/// Parse a binding string like "RightCtrl" or "Ctrl+Alt+Space" into a set of
/// evdev KeyCodes. The session is open while ALL keys in the set are held, and
/// closes the moment ANY key in the set is released.
fn parse_binding(s: &str) -> Result<HashSet<KeyCode>> {
    let mut keys = HashSet::new();
    for part in s.split('+').map(str::trim) {
        let k = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "leftctrl" => KeyCode::KEY_LEFTCTRL,
            "rightctrl" => KeyCode::KEY_RIGHTCTRL,
            "shift" | "leftshift" => KeyCode::KEY_LEFTSHIFT,
            "rightshift" => KeyCode::KEY_RIGHTSHIFT,
            "alt" | "leftalt" => KeyCode::KEY_LEFTALT,
            "rightalt" => KeyCode::KEY_RIGHTALT,
            "meta" | "super" | "leftmeta" => KeyCode::KEY_LEFTMETA,
            "rightmeta" => KeyCode::KEY_RIGHTMETA,
            "space" => KeyCode::KEY_SPACE,
            "f1" => KeyCode::KEY_F1,
            "f2" => KeyCode::KEY_F2,
            "f3" => KeyCode::KEY_F3,
            "f4" => KeyCode::KEY_F4,
            "f5" => KeyCode::KEY_F5,
            "f6" => KeyCode::KEY_F6,
            "f7" => KeyCode::KEY_F7,
            "f8" => KeyCode::KEY_F8,
            "f9" => KeyCode::KEY_F9,
            "f10" => KeyCode::KEY_F10,
            "f11" => KeyCode::KEY_F11,
            "f12" => KeyCode::KEY_F12,
            other => anyhow::bail!("unsupported hotkey key: {other}"),
        };
        keys.insert(k);
    }
    if keys.is_empty() {
        anyhow::bail!("empty binding");
    }
    Ok(keys)
}

fn enumerate_keyboards() -> Vec<(PathBuf, Device)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        for e in entries.flatten() {
            let p = e.path();
            if !p.to_string_lossy().contains("event") {
                continue;
            }
            if let Ok(dev) = Device::open(&p) {
                if dev
                    .supported_keys()
                    .map(|k| k.contains(KeyCode::KEY_A))
                    .unwrap_or(false)
                {
                    out.push((p, dev));
                }
            }
        }
    }
    out
}

pub fn listen(binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
    let wanted = parse_binding(binding).with_context(|| format!("invalid binding: {binding:?}"))?;
    let (tx, rx) = mpsc::channel(16);

    thread::spawn(move || loop {
        let kbds = enumerate_keyboards();
        if kbds.is_empty() {
            tracing::warn!("no keyboards visible via /dev/input/event*; retrying in 2s");
            thread::sleep(std::time::Duration::from_secs(2));
            continue;
        }
        if let Err(e) = run_loop(&wanted, kbds, &tx) {
            tracing::warn!("hotkey loop ended: {e}; reacquiring in 2s");
            thread::sleep(std::time::Duration::from_secs(2));
        }
    });

    Ok(rx)
}

fn run_loop(
    wanted: &HashSet<KeyCode>,
    mut kbds: Vec<(PathBuf, Device)>,
    tx: &mpsc::Sender<HotkeyEvent>,
) -> Result<()> {
    let mut held: HashSet<KeyCode> = HashSet::new();
    let mut session_open = false;
    loop {
        for (_, dev) in kbds.iter_mut() {
            for ev in dev.fetch_events()? {
                let key_code = match ev.destructure() {
                    EventSummary::Key(_, kc, _) => kc,
                    _ => continue,
                };
                if !wanted.contains(&key_code) {
                    continue;
                }
                match ev.value() {
                    1 => {
                        held.insert(key_code);
                    }
                    2 => {} // auto-repeat, ignore
                    0 => {
                        held.remove(&key_code);
                    }
                    _ => {}
                }
                let all_held = wanted.is_subset(&held);
                if all_held && !session_open {
                    session_open = true;
                    let _ = tx.blocking_send(HotkeyEvent::Press);
                } else if !all_held && session_open {
                    session_open = false;
                    let _ = tx.blocking_send(HotkeyEvent::Release);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_key() {
        let k = parse_binding("RightCtrl").unwrap();
        assert!(k.contains(&KeyCode::KEY_RIGHTCTRL));
        assert_eq!(k.len(), 1);
    }

    #[test]
    fn parses_chord() {
        let k = parse_binding("Ctrl+Alt+Space").unwrap();
        assert_eq!(k.len(), 3);
        assert!(k.contains(&KeyCode::KEY_LEFTCTRL));
        assert!(k.contains(&KeyCode::KEY_LEFTALT));
        assert!(k.contains(&KeyCode::KEY_SPACE));
    }

    #[test]
    fn parses_function_key() {
        let k = parse_binding("F8").unwrap();
        assert!(k.contains(&KeyCode::KEY_F8));
    }

    #[test]
    fn rejects_unknown() {
        assert!(parse_binding("FlibberHotkey").is_err());
    }
}
