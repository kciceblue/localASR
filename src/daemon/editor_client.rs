//! Client for OpenAI-compatible `/v1/chat/completions`, used twice:
//!  - light pass: per VAD chunk, polishes the last few chunks
//!  - heavy pass: on session close, polishes the whole utterance with optional term DB

use crate::config::EditorConfig;
use crate::terms::TermDb;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_LIGHT_PROMPT: &str = "You polish recent dictation. Fix obvious ASR errors only (mis-heard words, capitalisation). Do not add or remove content. Reply with the corrected text for the last chunk and nothing else.";

const DEFAULT_HEAVY_PROMPT: &str = "You polish a dictated utterance. Fix mis-heard words, punctuation, and capitalisation. Preserve meaning and length. If the speaker mentions a phrase that sounds like one of the listed terms, prefer the listed spelling.";

#[async_trait]
pub trait Editor: Send + Sync {
    async fn polish_light(&self, recent_chunks: &[String]) -> Result<String>;
    async fn polish_heavy(&self, utterance: &str, terms: Option<&TermDb>) -> Result<String>;
}

pub struct OpenAiEditor {
    cfg: EditorConfig,
    light_http: reqwest::Client,
    heavy_http: reqwest::Client,
}

impl OpenAiEditor {
    pub fn new(cfg: EditorConfig) -> Result<Self> {
        let light_http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.light_timeout_ms))
            .build()?;
        let heavy_http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.heavy_timeout_ms))
            .build()?;
        Ok(Self { cfg, light_http, heavy_http })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    temperature: f32,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMsg,
}

#[derive(Deserialize)]
struct ChoiceMsg {
    content: String,
}

#[async_trait]
impl Editor for OpenAiEditor {
    async fn polish_light(&self, recent_chunks: &[String]) -> Result<String> {
        let system_prompt = if self.cfg.light.system_prompt.is_empty() {
            DEFAULT_LIGHT_PROMPT.to_string()
        } else {
            self.cfg.light.system_prompt.clone()
        };
        let user = format!(
            "Recent chunks (oldest first):\n---\n{}\n---\nReturn only the corrected text for the LAST chunk.",
            recent_chunks.join("\n")
        );
        let req = ChatRequest {
            model: &self.cfg.model,
            temperature: self.cfg.light_temperature,
            messages: vec![
                Message { role: "system", content: &system_prompt },
                Message { role: "user", content: &user },
            ],
        };
        send(&self.light_http, &self.cfg.base_url, &self.cfg.api_key, &req).await
    }

    async fn polish_heavy(&self, utterance: &str, terms: Option<&TermDb>) -> Result<String> {
        let mut system_prompt = if self.cfg.heavy.system_prompt.is_empty() {
            DEFAULT_HEAVY_PROMPT.to_string()
        } else {
            self.cfg.heavy.system_prompt.clone()
        };
        if let Some(db) = terms {
            if !db.entries.is_empty() {
                system_prompt.push_str("\n\nTerms to prefer:\n");
                system_prompt.push_str(&crate::terms::render_prompt_block(db));
            }
        }
        let req = ChatRequest {
            model: &self.cfg.model,
            temperature: self.cfg.heavy_temperature,
            messages: vec![
                Message { role: "system", content: &system_prompt },
                Message { role: "user", content: utterance },
            ],
        };
        send(&self.heavy_http, &self.cfg.base_url, &self.cfg.api_key, &req).await
    }
}

async fn send(http: &reqwest::Client, base: &str, key: &str, req: &ChatRequest<'_>) -> Result<String> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = http
        .post(&url)
        .bearer_auth(key)
        .json(req)
        .send()
        .await
        .context("editor HTTP request failed")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("editor endpoint returned {}: {}", status, body);
    }
    let parsed: ChatResponse = resp.json().await.context("editor JSON parse")?;
    let text = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default();
    Ok(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EditorPassConfig;
    use crate::terms::Term;
    use wiremock::matchers::{body_string_contains, method, path};
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
    async fn light_pass_returns_polished_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ok_response("hello world."))
            .mount(&server)
            .await;
        let editor = OpenAiEditor::new(cfg(&server.uri())).unwrap();
        let out = editor.polish_light(&["hello wrold".into()]).await.unwrap();
        assert_eq!(out, "hello world.");
    }

    #[tokio::test]
    async fn heavy_pass_without_terms_works() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ok_response("polished."))
            .mount(&server)
            .await;
        let editor = OpenAiEditor::new(cfg(&server.uri())).unwrap();
        let out = editor.polish_heavy("rough.", None).await.unwrap();
        assert_eq!(out, "polished.");
    }

    #[tokio::test]
    async fn heavy_pass_includes_terms_in_system_prompt() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("kubectl"))
            .respond_with(ok_response("use kubectl"))
            .mount(&server)
            .await;
        let editor = OpenAiEditor::new(cfg(&server.uri())).unwrap();
        let db = TermDb {
            entries: vec![Term {
                name: "kubectl".into(),
                aliases: vec!["cube control".into()],
                hint: "k8s cli".into(),
            }],
        };
        let out = editor.polish_heavy("use cube control", Some(&db)).await.unwrap();
        assert_eq!(out, "use kubectl");
    }
}
