//! Client for OpenAI-compatible `/v1/audio/transcriptions`.

use crate::config::AsrConfig;
use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use std::time::Duration;

#[async_trait]
pub trait Asr: Send + Sync {
    /// Transcribe 16kHz mono i16 PCM. Returns just the text.
    async fn transcribe(&self, samples: &[i16]) -> Result<String>;
}

pub struct OpenAiAsr {
    cfg: AsrConfig,
    http: reqwest::Client,
}

impl OpenAiAsr {
    pub fn new(cfg: AsrConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .build()?;
        Ok(Self { cfg, http })
    }
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[async_trait]
impl Asr for OpenAiAsr {
    async fn transcribe(&self, samples: &[i16]) -> Result<String> {
        let wav = encode_wav(samples, 16000)?;
        let url = format!("{}/audio/transcriptions", self.cfg.base_url.trim_end_matches('/'));

        let mut form = reqwest::multipart::Form::new()
            .text("model", self.cfg.model.clone())
            .text("response_format", "json")
            .part(
                "file",
                reqwest::multipart::Part::bytes(Bytes::from(wav).to_vec())
                    .file_name("audio.wav")
                    .mime_str("audio/wav")?,
            );
        if !self.cfg.language.is_empty() {
            form = form.text("language", self.cfg.language.clone());
        }

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .multipart(form)
            .send()
            .await
            .context("ASR HTTP request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("ASR endpoint returned {}: {}", status, body);
        }
        let parsed: TranscriptionResponse = resp.json().await.context("ASR JSON parse")?;
        Ok(parsed.text)
    }
}

/// Encode 16kHz mono i16 PCM as a WAV byte buffer.
fn encode_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cursor, spec)?;
        for s in samples {
            w.write_sample(*s)?;
        }
        w.finalize()?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
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
    async fn happy_path_returns_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "text": "hello world"
            })))
            .mount(&server)
            .await;
        let client = OpenAiAsr::new(cfg(&server.uri())).unwrap();
        let out = client.transcribe(&vec![0i16; 16000]).await.unwrap();
        assert_eq!(out, "hello world");
    }

    #[tokio::test]
    async fn server_error_propagates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let client = OpenAiAsr::new(cfg(&server.uri())).unwrap();
        let err = client.transcribe(&vec![0i16; 1600]).await.unwrap_err();
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn timeout_propagates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&server)
            .await;
        let mut c = cfg(&server.uri());
        c.timeout_ms = 50;
        let client = OpenAiAsr::new(c).unwrap();
        assert!(client.transcribe(&vec![0i16; 1600]).await.is_err());
    }
}
