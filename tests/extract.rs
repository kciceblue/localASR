//! Integration test for `localasr::extract::run`. Stands up a wiremock server
//! that returns a canned glossary response, points `extract` at a tempdir
//! containing a tiny file, and asserts the produced terms.toml.

use localasr::config::{EditorConfig, EditorPassConfig};
use std::path::PathBuf;
use tempfile::TempDir;
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
        heavy_timeout_ms: 5000,
        light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
        heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
    }
}

fn ok_json(content: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": content } }]
    }))
}

#[tokio::test]
async fn extract_end_to_end_writes_terms_toml() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_json(r#"{"terms":[{"name":"kubectl","aliases":["cube control"],"hint":"k8s cli"},{"name":"tokio","aliases":[],"hint":"rust async"}]}"#))
        .mount(&server)
        .await;

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("a.rs"), b"use tokio;\nuse kubectl;\n").unwrap();
    std::fs::write(src.path().join("README.md"), b"# notes about kubectl\n").unwrap();

    let out_dir = TempDir::new().unwrap();
    let out = out_dir.path().join("terms.toml");

    let chunks = localasr::extract::run(src.path(), &out, cfg(&server.uri()))
        .await
        .unwrap();
    assert!(chunks >= 2, "expected at least one chunk per file, got {chunks}");

    let db = localasr::terms::load(&out).unwrap();
    let names: Vec<&str> = db.entries.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["kubectl", "tokio"]);
    let kubectl = &db.entries[0];
    assert_eq!(kubectl.aliases, vec!["cube control"]);
    assert_eq!(kubectl.hint, "k8s cli");
}

#[tokio::test]
async fn extract_silently_skips_per_chunk_failures() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("a.rs"), b"hi\n").unwrap();
    let out_dir = TempDir::new().unwrap();
    let out = out_dir.path().join("terms.toml");
    localasr::extract::run(src.path(), &out, cfg(&server.uri())).await.unwrap();
    let db = localasr::terms::load(&out).unwrap();
    assert!(db.entries.is_empty());
}

#[tokio::test]
async fn extract_empty_folder_writes_empty_terms_toml() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_json(r#"{"terms":[]}"#))
        .mount(&server)
        .await;
    let src = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();
    let out = out_dir.path().join("terms.toml");
    let chunks = localasr::extract::run(src.path(), &out, cfg(&server.uri())).await.unwrap();
    assert_eq!(chunks, 0);
    assert!(out.exists());
}

#[tokio::test]
async fn extract_missing_folder_errors() {
    let out = PathBuf::from("/tmp/this-doesnt-matter.toml");
    let cfg_dummy = cfg("http://127.0.0.1:9");
    let err = localasr::extract::run(std::path::Path::new("/nonexistent-9d8a2"), &out, cfg_dummy)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"));
}
