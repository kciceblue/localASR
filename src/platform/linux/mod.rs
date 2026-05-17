pub mod clipboard;
pub mod hotkey;   // Task 11
pub mod input;    // Task 10

use crate::platform::{ClipboardSnapshot, HotkeyEvent, Platform};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

pub struct LinuxPlatform;

impl LinuxPlatform {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Platform for LinuxPlatform {
    fn hotkey_stream(self: Arc<Self>, binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
        hotkey::listen(binding)
    }
    async fn read_clipboard(&self) -> Result<ClipboardSnapshot> {
        task::spawn_blocking(clipboard::read).await?
    }
    async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()> {
        let s = snap.clone();
        task::spawn_blocking(move || clipboard::write(&s)).await?
    }
    async fn paste(&self, shortcut: &str) -> Result<()> {
        let s = shortcut.to_string();
        task::spawn_blocking(move || input::paste(&s)).await?
    }
    async fn backspace(&self, n: usize) -> Result<()> {
        task::spawn_blocking(move || input::backspace(n)).await?
    }
}
