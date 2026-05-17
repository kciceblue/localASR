//! Cross-platform abstraction for hotkey capture and synthetic input.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
pub mod linux;
// FIXME(Task 13): pub mod windows;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HotkeyEvent {
    Press,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClipboardSnapshot(pub Option<String>);

/// Platform-facing capabilities used by the daemon. All methods may run on
/// blocking threads internally; the trait surface is async so the pipeline can
/// `.await` them.
#[async_trait]
pub trait Platform: Send + Sync {
    /// Begin listening for the configured hotkey. Returns a receiver that yields
    /// Press/Release events. The internal thread runs for the lifetime of the
    /// returned receiver; dropping the receiver stops the thread.
    fn hotkey_stream(self: Arc<Self>, binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>>;

    async fn read_clipboard(&self) -> Result<ClipboardSnapshot>;
    async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()>;

    /// Send the paste keystroke (e.g. "Ctrl+V").
    async fn paste(&self, shortcut: &str) -> Result<()>;

    /// Send N backspaces.
    async fn backspace(&self, n: usize) -> Result<()>;
}

/// In-memory platform for tests. Always exported (cheap) so integration tests in
/// `tests/` can use it without a feature flag.
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, PartialEq, Eq, Clone)]
    pub enum Call {
        ReadClipboard,
        WriteClipboard(ClipboardSnapshot),
        Paste(String),
        Backspace(usize),
    }

    #[derive(Default)]
    pub struct MockPlatform {
        pub clipboard: Mutex<ClipboardSnapshot>,
        pub calls: Mutex<Vec<Call>>,
    }

    impl MockPlatform {
        pub fn new() -> Arc<Self> { Arc::new(Self::default()) }
        pub fn calls(&self) -> Vec<Call> { self.calls.lock().unwrap().clone() }
    }

    #[async_trait]
    impl Platform for MockPlatform {
        fn hotkey_stream(self: Arc<Self>, _binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
            let (_tx, rx) = mpsc::channel(8);
            Ok(rx)
        }
        async fn read_clipboard(&self) -> Result<ClipboardSnapshot> {
            self.calls.lock().unwrap().push(Call::ReadClipboard);
            Ok(self.clipboard.lock().unwrap().clone())
        }
        async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()> {
            self.calls.lock().unwrap().push(Call::WriteClipboard(snap.clone()));
            *self.clipboard.lock().unwrap() = snap.clone();
            Ok(())
        }
        async fn paste(&self, shortcut: &str) -> Result<()> {
            self.calls.lock().unwrap().push(Call::Paste(shortcut.into()));
            Ok(())
        }
        async fn backspace(&self, n: usize) -> Result<()> {
            self.calls.lock().unwrap().push(Call::Backspace(n));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;

    #[tokio::test]
    async fn mock_records_calls_in_order() {
        let p = MockPlatform::new();
        p.write_clipboard(&ClipboardSnapshot(Some("hi".into()))).await.unwrap();
        p.paste("Ctrl+V").await.unwrap();
        p.backspace(3).await.unwrap();
        assert_eq!(
            p.calls(),
            vec![
                Call::WriteClipboard(ClipboardSnapshot(Some("hi".into()))),
                Call::Paste("Ctrl+V".into()),
                Call::Backspace(3),
            ]
        );
    }
}
