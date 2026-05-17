//! Individual diagnostic probes. Each probe is an async fn returning `Result<()>`.
//! Implementations land in Tasks 2–6.

use anyhow::{Context, Result};
use std::path::PathBuf;
use crate::config::AsrConfig;
use crate::daemon::asr_client::{Asr, OpenAiAsr};

/// Probe: the config file exists at the given path and parses successfully.
pub async fn config_probe(path: PathBuf) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("no config file at {}", path.display());
    }
    crate::config::load(&path)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(())
}

/// Probe: send 0.5s of silence to the ASR endpoint and verify a 2xx response.
/// The returned text is allowed to be empty (silence -> empty is fine).
pub async fn asr_probe(cfg: AsrConfig) -> Result<()> {
    let client = OpenAiAsr::new(cfg).context("constructing ASR client")?;
    let silence = vec![0i16; 8000]; // 0.5s at 16kHz
    client.transcribe(&silence).await.context("ASR endpoint round-trip")?;
    Ok(())
}

#[cfg(test)]
mod asr_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg(base: &str) -> AsrConfig {
        AsrConfig {
            base_url: base.into(),
            api_key: "sk-test".into(),
            model: "whisper-1".into(),
            language: "".into(),
            timeout_ms: 5000,
        }
    }

    #[tokio::test]
    async fn asr_probe_passes_on_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": ""})))
            .mount(&server).await;
        assert!(asr_probe(cfg(&server.uri())).await.is_ok());
    }

    #[tokio::test]
    async fn asr_probe_fails_on_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server).await;
        assert!(asr_probe(cfg(&server.uri())).await.is_err());
    }
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

use crate::config::EditorConfig;
use crate::daemon::editor_client::{Editor, OpenAiEditor};

/// Probe: ask the editor endpoint to polish a trivial string. Verify a 2xx
/// response and a non-empty completion. Term DB is intentionally not exercised
/// here — that lands as a separate probe in Plan 4 with the `extract` work.
pub async fn editor_probe(cfg: EditorConfig) -> Result<()> {
    let client = OpenAiEditor::new(cfg).context("constructing editor client")?;
    let out = client.polish_heavy("hello", None).await.context("editor endpoint round-trip")?;
    if out.is_empty() {
        anyhow::bail!("editor returned an empty completion");
    }
    Ok(())
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use crate::config::EditorPassConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg(base: &str) -> EditorConfig {
        EditorConfig {
            base_url: base.into(),
            api_key: "sk-test".into(),
            model: "gpt-4o-mini".into(),
            light_temperature: 0.0,
            heavy_temperature: 0.2,
            light_timeout_ms: 3000,
            heavy_timeout_ms: 10000,
            light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
            heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
        }
    }

    fn ok_response(text: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": text } }]
        }))
    }

    #[tokio::test]
    async fn editor_probe_passes_on_non_empty_completion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ok_response("hi back."))
            .mount(&server).await;
        assert!(editor_probe(cfg(&server.uri())).await.is_ok());
    }

    #[tokio::test]
    async fn editor_probe_fails_on_empty_completion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ok_response(""))
            .mount(&server).await;
        let err = editor_probe(cfg(&server.uri())).await.unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[tokio::test]
    async fn editor_probe_fails_on_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server).await;
        assert!(editor_probe(cfg(&server.uri())).await.is_err());
    }
}
