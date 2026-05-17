use anyhow::{Context, Result};
use evdev::{
    uinput::VirtualDevice,
    AttributeSet, EventType, InputEvent, KeyCode,
};
use std::sync::Mutex;
use std::time::Duration;

static DEVICE: Mutex<Option<VirtualDevice>> = Mutex::new(None);

fn device() -> Result<std::sync::MutexGuard<'static, Option<VirtualDevice>>> {
    let mut g = DEVICE.lock().unwrap();
    if g.is_none() {
        let mut keys = AttributeSet::<KeyCode>::new();
        for k in ALL_USED_KEYS {
            keys.insert(*k);
        }
        let dev = VirtualDevice::builder()
            .context("creating uinput device; ensure /dev/uinput is writable (see 99-localasr.rules)")?
            .name("localasr-virtual-kbd")
            .with_keys(&keys)?
            .build()?;
        *g = Some(dev);
        // Give udev a moment to set up the device node
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(g)
}

const ALL_USED_KEYS: &[KeyCode] = &[
    KeyCode::KEY_LEFTCTRL, KeyCode::KEY_RIGHTCTRL,
    KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTSHIFT,
    KeyCode::KEY_LEFTALT, KeyCode::KEY_RIGHTALT,
    KeyCode::KEY_LEFTMETA, KeyCode::KEY_RIGHTMETA,
    KeyCode::KEY_V, KeyCode::KEY_BACKSPACE,
];

fn parse_chord(s: &str) -> Result<Vec<KeyCode>> {
    let mut keys = Vec::new();
    for part in s.split('+').map(str::trim) {
        let k = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "leftctrl" => KeyCode::KEY_LEFTCTRL,
            "rightctrl" => KeyCode::KEY_RIGHTCTRL,
            "shift" | "leftshift" => KeyCode::KEY_LEFTSHIFT,
            "rightshift" => KeyCode::KEY_RIGHTSHIFT,
            "alt" | "leftalt" => KeyCode::KEY_LEFTALT,
            "rightalt" => KeyCode::KEY_RIGHTALT,
            "meta" | "super" | "leftmeta" => KeyCode::KEY_LEFTMETA,
            "rightmeta" => KeyCode::KEY_RIGHTMETA,
            "v" => KeyCode::KEY_V,
            "backspace" | "back" => KeyCode::KEY_BACKSPACE,
            other => anyhow::bail!("unsupported key in chord: {other}"),
        };
        keys.push(k);
    }
    if keys.is_empty() {
        anyhow::bail!("empty chord");
    }
    Ok(keys)
}

fn tap_chord(keys: &[KeyCode]) -> Result<()> {
    let mut g = device()?;
    let dev = g.as_mut().unwrap();
    let key_type = EventType::KEY.0;
    let down: Vec<InputEvent> = keys.iter()
        .map(|k| InputEvent::new(key_type, k.0, 1))
        .collect();
    let up: Vec<InputEvent> = keys.iter().rev()
        .map(|k| InputEvent::new(key_type, k.0, 0))
        .collect();
    dev.emit(&down)?;
    std::thread::sleep(Duration::from_millis(5));
    dev.emit(&up)?;
    Ok(())
}

pub fn paste(shortcut: &str) -> Result<()> {
    let keys = parse_chord(shortcut)?;
    tap_chord(&keys)
}

pub fn backspace(n: usize) -> Result<()> {
    let keys = vec![KeyCode::KEY_BACKSPACE];
    for _ in 0..n {
        tap_chord(&keys)?;
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_key() {
        let k = parse_chord("V").unwrap();
        assert_eq!(k, vec![KeyCode::KEY_V]);
    }

    #[test]
    fn parses_chord() {
        let k = parse_chord("Ctrl+V").unwrap();
        assert_eq!(k, vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V]);
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_chord("XYZ").is_err());
    }
}
