use crate::platform::HotkeyEvent;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Mutex;
use std::thread;
use tokio::sync::mpsc;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
    VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

struct State {
    wanted: HashSet<u16>,
    held: HashSet<u16>,
    open: bool,
    tx: mpsc::Sender<HotkeyEvent>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn parse_binding(s: &str) -> Result<HashSet<u16>> {
    let mut keys = HashSet::new();
    for part in s.split('+').map(str::trim) {
        let vk: VIRTUAL_KEY = match part.to_ascii_lowercase().as_str() {
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
            "f1" => VK_F1, "f2" => VK_F2, "f3" => VK_F3, "f4" => VK_F4,
            "f5" => VK_F5, "f6" => VK_F6, "f7" => VK_F7, "f8" => VK_F8,
            "f9" => VK_F9, "f10" => VK_F10, "f11" => VK_F11, "f12" => VK_F12,
            c if c.len() == 1 => VIRTUAL_KEY(c.chars().next().unwrap().to_ascii_uppercase() as u16),
            other => anyhow::bail!("unsupported hotkey: {other}"),
        };
        keys.insert(vk.0);
    }
    Ok(keys)
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kbd = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // vkCode is u32 in windows 0.59
        let vk = kbd.vkCode as u16;
        let msg = wparam.0 as u32;
        let mut guard = STATE.lock().unwrap();
        if let Some(st) = guard.as_mut() {
            if st.wanted.contains(&vk) {
                match msg {
                    m if m == WM_KEYDOWN || m == WM_SYSKEYDOWN => { st.held.insert(vk); }
                    m if m == WM_KEYUP || m == WM_SYSKEYUP => { st.held.remove(&vk); }
                    _ => {}
                }
                let all = st.wanted.is_subset(&st.held);
                if all && !st.open {
                    st.open = true;
                    let _ = st.tx.try_send(HotkeyEvent::Press);
                } else if !all && st.open {
                    st.open = false;
                    let _ = st.tx.try_send(HotkeyEvent::Release);
                }
            }
        }
    }
    // CallNextHookEx takes Option<HHOOK> in windows 0.59
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub fn listen(binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
    let wanted = parse_binding(binding)?;
    let (tx, rx) = mpsc::channel(16);
    *STATE.lock().unwrap() = Some(State { wanted, held: HashSet::new(), open: false, tx });

    thread::spawn(|| unsafe {
        // GetModuleHandleW returns Result<HMODULE>; convert to HINSTANCE via From impl
        let hmod: Option<HINSTANCE> = GetModuleHandleW(None)
            .ok()
            .map(HINSTANCE::from);
        let hook: HHOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0)
            .expect("SetWindowsHookExW failed");
        let mut msg = MSG::default();
        // GetMessageW returns BOOL; use as_bool() to check loop condition
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_chord() {
        let k = parse_binding("Ctrl+Alt+Space").unwrap();
        assert_eq!(k.len(), 3);
    }
}
