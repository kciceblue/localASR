use crate::platform::HotkeyEvent;
use anyhow::Result;
use tokio::sync::mpsc;

pub fn listen(_binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
    anyhow::bail!("Linux hotkey listener not implemented yet (Task 11)")
}
