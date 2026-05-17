//! Per-chunk glossary extractor: posts a chat-completion request to the
//! configured editor endpoint with a JSON-output system prompt, parses the
//! response into `Vec<Term>`.
//!
//! Output contract: the model is asked to return ONLY a JSON object of the
//! form `{"terms": [{"name": "...", "aliases": ["..."], "hint": "..."}]}`.
//! We strip code fences if present and then parse. If parsing fails the
//! chunk yields zero terms (logged at WARN); we don't abort the whole run.

use crate::config::EditorConfig;
use crate::terms::Term;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const SYSTEM_PROMPT: &str = r#"You are a glossary extractor for a dictation system.

Given a chunk of text (source code, docs, notes, etc.), identify domain-specific terms that a dictation speech-to-text system would otherwise mis-hear: tool names, library names, proper nouns, acronyms, technical jargon.

For each term, emit:
- "name": the canonical spelling
- "aliases": an array of common phonetic mis-hearings (be CONSERVATIVE; only include if you can think of a plausible mis-hearing — empty array is fine)
- "hint": a short context, max 80 characters (empty string is fine)

Output STRICT JSON only, no prose, no markdown fences. Schema:
{"terms": [{"name": "kubectl", "aliases": ["cube control"], "hint": "Kubernetes CLI"}]}

Skip common English words. Aim for 5-20 terms per chunk; quality over quantity. If the chunk has no domain terms, return {"terms": []}."#;

pub struct ExtractClient {
    cfg: EditorConfig,
    http: reqwest::Client,
}

impl ExtractClient {
    pub fn new(cfg: EditorConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.heavy_timeout_ms))
            .build()
            .context("build reqwest client for extract")?;
        Ok(Self { cfg, http })
    }

    /// Extract terms from a single chunk. `source_hint` is a human-readable
    /// label (e.g. file path + chunk index) embedded in the user message so
    /// the model can ground its choices.
    pub async fn extract(&self, chunk: &str, source_hint: &str) -> Result<Vec<Term>> {
        let user = format!("Source: {source_hint}\n\nText:\n---\n{chunk}\n---");
        let req = ChatRequest {
            model: &self.cfg.model,
            temperature: 0.0,
            messages: vec![
                Message { role: "system", content: SYSTEM_PROMPT },
                Message { role: "user", content: &user },
            ],
        };
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .json(&req)
            .send()
            .await
            .context("extract HTTP request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("extract endpoint returned {status}: {body}");
        }
        let parsed: ChatResponse = resp.json().await.context("extract JSON parse")?;
        let raw = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();
        Ok(parse_terms(&raw))
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

#[derive(Deserialize)]
struct ExtractPayload {
    #[serde(default)]
    terms: Vec<RawTerm>,
}

#[derive(Deserialize)]
struct RawTerm {
    #[serde(default)]
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    hint: String,
}

/// Parse the model's reply into `Term`s. Tolerant of:
/// - Surrounding whitespace / newlines.
/// - Markdown code fences (```json ... ``` or ``` ... ```).
/// - Missing fields (filled with empty defaults).
///
/// Returns empty Vec on unparseable input.
fn parse_terms(raw: &str) -> Vec<Term> {
    let cleaned = strip_code_fence(raw.trim());
    let payload: ExtractPayload = match serde_json::from_str(cleaned) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                "extract: JSON parse failed: {e} | raw start: {:?}",
                &cleaned.chars().take(80).collect::<String>()
            );
            return Vec::new();
        }
    };
    payload
        .terms
        .into_iter()
        .filter(|t| !t.name.is_empty())
        .map(|t| Term { name: t.name, aliases: t.aliases, hint: t.hint })
        .collect()
}

/// Strip a leading ```lang and trailing ``` fence if present. Idempotent.
fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
        // Skip optional language tag up to first newline.
        let after_tag = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or(rest);
        let trimmed = after_tag.trim_end();
        if let Some(without_suffix) = trimmed.strip_suffix("```") {
            return without_suffix.trim();
        }
        return after_tag.trim();
    }
    s
}

#[cfg(test)]
mod tests {
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
            heavy_temperature: 0.0,
            light_timeout_ms: 5000,
            heavy_timeout_ms: 15000,
            light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
            heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
        }
    }

    fn ok_response(content: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": content } }]
        }))
    }

    #[test]
    fn parse_terms_happy_path() {
        let raw = r#"{"terms":[{"name":"kubectl","aliases":["cube control"],"hint":"k8s cli"}]}"#;
        let t = parse_terms(raw);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "kubectl");
        assert_eq!(t[0].aliases, vec!["cube control"]);
    }

    #[test]
    fn parse_terms_strips_fences() {
        let raw = "```json\n{\"terms\":[{\"name\":\"tokio\",\"aliases\":[],\"hint\":\"\"}]}\n```";
        let t = parse_terms(raw);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "tokio");
    }

    #[test]
    fn parse_terms_missing_fields_defaults_empty() {
        let raw = r#"{"terms":[{"name":"x"}]}"#;
        let t = parse_terms(raw);
        assert_eq!(t.len(), 1);
        assert!(t[0].aliases.is_empty());
        assert_eq!(t[0].hint, "");
    }

    #[test]
    fn parse_terms_drops_empty_names() {
        let raw = r#"{"terms":[{"name":""},{"name":"keep"}]}"#;
        let t = parse_terms(raw);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "keep");
    }

    #[test]
    fn parse_terms_unparseable_returns_empty() {
        assert!(parse_terms("not json at all").is_empty());
        assert!(parse_terms("").is_empty());
    }

    #[test]
    fn strip_code_fence_no_fence_passes_through() {
        assert_eq!(strip_code_fence(r#"{"terms":[]}"#), r#"{"terms":[]}"#);
    }

    #[tokio::test]
    async fn extract_happy_path_returns_terms() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ok_response(r#"{"terms":[{"name":"reqwest","aliases":["request"],"hint":"Rust HTTP client"}]}"#))
            .mount(&server)
            .await;
        let client = ExtractClient::new(cfg(&server.uri())).unwrap();
        let out = client.extract("use reqwest;", "src/lib.rs:0").await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "reqwest");
    }

    #[tokio::test]
    async fn extract_server_error_propagates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let client = ExtractClient::new(cfg(&server.uri())).unwrap();
        let err = client.extract("x", "f:0").await.unwrap_err();
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn extract_garbage_response_yields_empty_no_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ok_response("I refuse to follow the format."))
            .mount(&server)
            .await;
        let client = ExtractClient::new(cfg(&server.uri())).unwrap();
        let out = client.extract("x", "f:0").await.unwrap();
        assert!(out.is_empty());
    }
}
