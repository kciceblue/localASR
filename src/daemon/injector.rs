//! Combines clipboard save/restore + paste + backspace into single high-level
//! operations. Exists as its own module so the orchestration is independently
//! testable against `MockPlatform`.

use crate::config::InjectionConfig;
use crate::daemon::state::Diff;
use crate::platform::{ClipboardSnapshot, Platform};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

pub struct Injector {
    cfg: InjectionConfig,
    platform: Arc<dyn Platform>,
}

impl Injector {
    pub fn new(cfg: InjectionConfig, platform: Arc<dyn Platform>) -> Self {
        Self { cfg, platform }
    }

    /// Apply a single diff: backspace N times, then paste the new tail (if any).
    /// `saved` is the clipboard snapshot we restore to afterwards.
    pub async fn apply(&self, diff: &Diff, saved: &ClipboardSnapshot) -> Result<()> {
        if diff.backspace_count == 0 && diff.append.is_empty() {
            return Ok(());
        }
        if diff.backspace_count > 0 {
            self.platform.backspace(diff.backspace_count).await?;
        }
        if !diff.append.is_empty() {
            self.platform.write_clipboard(&ClipboardSnapshot(Some(diff.append.clone()))).await?;
            self.platform.paste(&self.cfg.paste_shortcut).await?;
            sleep(Duration::from_millis(self.cfg.restore_delay_ms)).await;
            self.platform.write_clipboard(saved).await?;
        }
        Ok(())
    }

    pub async fn save_clipboard(&self) -> Result<ClipboardSnapshot> {
        self.platform.read_clipboard().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::{Call, MockPlatform};

    fn cfg() -> InjectionConfig {
        InjectionConfig {
            mode: "clipboard_paste".into(),
            paste_shortcut: "Ctrl+V".into(),
            restore_delay_ms: 1,
        }
    }

    #[tokio::test]
    async fn append_only_pastes_and_restores() {
        let p = MockPlatform::new();
        let saved = ClipboardSnapshot(Some("original".into()));
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 0, append: "hello".into() }, &saved).await.unwrap();
        let calls = p.calls();
        assert_eq!(calls[0], Call::WriteClipboard(ClipboardSnapshot(Some("hello".into()))));
        assert_eq!(calls[1], Call::Paste("Ctrl+V".into()));
        assert_eq!(calls[2], Call::WriteClipboard(saved));
    }

    #[tokio::test]
    async fn backspace_only_does_not_touch_clipboard() {
        let p = MockPlatform::new();
        let saved = ClipboardSnapshot(Some("x".into()));
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 3, append: "".into() }, &saved).await.unwrap();
        let calls = p.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], Call::Backspace(3));
    }

    #[tokio::test]
    async fn backspace_then_paste() {
        let p = MockPlatform::new();
        let saved = ClipboardSnapshot(None);
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 5, append: "world".into() }, &saved).await.unwrap();
        let calls = p.calls();
        assert_eq!(calls[0], Call::Backspace(5));
        assert_eq!(calls[1], Call::WriteClipboard(ClipboardSnapshot(Some("world".into()))));
        assert_eq!(calls[2], Call::Paste("Ctrl+V".into()));
        assert_eq!(calls[3], Call::WriteClipboard(saved));
    }

    #[tokio::test]
    async fn empty_diff_is_noop() {
        let p = MockPlatform::new();
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 0, append: "".into() }, &ClipboardSnapshot(None)).await.unwrap();
        assert!(p.calls().is_empty());
    }
}
