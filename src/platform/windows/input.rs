use anyhow::Result;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
    VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12,
};

fn parse_key(s: &str) -> Result<VIRTUAL_KEY> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "leftctrl" => VK_LCONTROL,
        "rightctrl" => VK_RCONTROL,
        "shift" => VK_SHIFT,
        "leftshift" => VK_LSHIFT,
        "rightshift" => VK_RSHIFT,
        "alt" => VK_MENU,
        "leftalt" => VK_LMENU,
        "rightalt" => VK_RMENU,
        "meta" | "super" | "leftmeta" => VK_LWIN,
        "rightmeta" => VK_RWIN,
        "space" => VK_SPACE,
        "backspace" | "back" => VK_BACK,
        "f1" => VK_F1, "f2" => VK_F2, "f3" => VK_F3, "f4" => VK_F4,
        "f5" => VK_F5, "f6" => VK_F6, "f7" => VK_F7, "f8" => VK_F8,
        "f9" => VK_F9, "f10" => VK_F10, "f11" => VK_F11, "f12" => VK_F12,
        c if c.len() == 1 => {
            let ch = c.chars().next().unwrap().to_ascii_uppercase();
            VIRTUAL_KEY(ch as u16)
        }
        other => anyhow::bail!("unsupported key: {other}"),
    })
}

fn key_event(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn tap_chord(parts: &[VIRTUAL_KEY]) -> Result<()> {
    let mut events: Vec<INPUT> = parts.iter().map(|k| key_event(*k, false)).collect();
    for k in parts.iter().rev() {
        events.push(key_event(*k, true));
    }
    unsafe {
        let sent = SendInput(&events, std::mem::size_of::<INPUT>() as i32);
        if sent as usize != events.len() {
            anyhow::bail!("SendInput sent {} of {} events", sent, events.len());
        }
    }
    std::thread::sleep(Duration::from_millis(2));
    Ok(())
}

pub fn paste(shortcut: &str) -> Result<()> {
    let keys: Result<Vec<_>> = shortcut.split('+').map(str::trim).map(parse_key).collect();
    tap_chord(&keys?)
}

pub fn backspace(n: usize) -> Result<()> {
    for _ in 0..n {
        tap_chord(&[VK_BACK])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ctrl_v() {
        let k = parse_key("Ctrl").unwrap();
        assert_eq!(k, VK_CONTROL);
        let k = parse_key("V").unwrap();
        assert_eq!(k.0, b'V' as u16);
    }
}
