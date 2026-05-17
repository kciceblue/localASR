//! Individual diagnostic probes. Each probe is an async fn returning `Result<()>`.
//! Implementations land in Tasks 2–6.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Probe: the config file exists at the given path and parses successfully.
pub async fn config_probe(path: PathBuf) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("no config file at {}", path.display());
    }
    crate::config::load(&path)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    const MINIMAL: &str = r#"
[hotkey]
binding = "RightCtrl"

[audio]
device = ""
sample_rate = 16000

[vad]
backend = "silero"
min_silence_ms = 400
max_chunk_ms = 5000

[asr]
base_url = "https://api.openai.com/v1"
api_key = "sk-test"
model = "whisper-1"
language = ""
timeout_ms = 15000

[editor]
base_url = "https://api.openai.com/v1"
api_key = "sk-test"
model = "gpt-4o-mini"
light_temperature = 0.0
heavy_temperature = 0.2
light_timeout_ms = 4000
heavy_timeout_ms = 15000

[editor.light]
enabled = true
context_chunks = 3
system_prompt = ""

[editor.heavy]
enabled = true
system_prompt = ""

[terms]
enabled = false
path = "/tmp/never-used.toml"

[injection]
mode = "clipboard_paste"
paste_shortcut = "Ctrl+V"
restore_delay_ms = 100
"#;

    #[tokio::test]
    async fn config_probe_passes_on_valid_file() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), MINIMAL).unwrap();
        assert!(config_probe(tmp.path().to_path_buf()).await.is_ok());
    }

    #[tokio::test]
    async fn config_probe_fails_on_missing_file() {
        let err = config_probe(PathBuf::from("/tmp/__definitely_does_not_exist__")).await.unwrap_err();
        assert!(err.to_string().contains("no config file"));
    }

    #[tokio::test]
    async fn config_probe_fails_on_invalid_toml() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "this is not toml]").unwrap();
        assert!(config_probe(tmp.path().to_path_buf()).await.is_err());
    }
}
