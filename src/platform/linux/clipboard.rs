use crate::platform::ClipboardSnapshot;
use anyhow::{Context, Result};

/// Read the current text clipboard contents.
pub fn read() -> Result<ClipboardSnapshot> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    match cb.get_text() {
        Ok(t) => Ok(ClipboardSnapshot(Some(t))),
        Err(arboard::Error::ContentNotAvailable) => Ok(ClipboardSnapshot(None)),
        Err(e) => Err(anyhow::anyhow!("reading clipboard: {e}")),
    }
}

/// Write the given snapshot. `None` clears the text payload (best-effort).
pub fn write(snap: &ClipboardSnapshot) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    match &snap.0 {
        Some(t) => cb.set_text(t.clone()).context("setting clipboard text")?,
        None => { let _ = cb.clear(); }
    }
    Ok(())
}
