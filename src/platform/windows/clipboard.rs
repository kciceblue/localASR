use crate::platform::ClipboardSnapshot;
use anyhow::{Context, Result};

pub fn read() -> Result<ClipboardSnapshot> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    match cb.get_text() {
        Ok(t) => Ok(ClipboardSnapshot(Some(t))),
        Err(arboard::Error::ContentNotAvailable) => Ok(ClipboardSnapshot(None)),
        Err(e) => Err(anyhow::anyhow!("reading clipboard: {e}")),
    }
}

pub fn write(snap: &ClipboardSnapshot) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening clipboard")?;
    if let Some(t) = &snap.0 {
        cb.set_text(t.clone()).context("setting clipboard")?;
    } else {
        let _ = cb.clear();
    }
    Ok(())
}
