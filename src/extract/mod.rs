//! `localasr extract <folder>` — generate a domain-specific terms database
//! by sending file chunks through the configured editor endpoint with a
//! glossary-extraction prompt.

pub mod chunker;
pub mod extractor;
pub mod merger;
pub mod scanner;

use crate::config::EditorConfig;
use crate::terms::TermDb;
use anyhow::{Context, Result};
use std::path::Path;

/// Top-level entry. Returns the number of chunks processed.
pub async fn run(folder: &Path, out_path: &Path, editor_cfg: EditorConfig) -> Result<usize> {
    let client = extractor::ExtractClient::new(editor_cfg)?;
    let mut db = TermDb { entries: Vec::new() };
    let mut chunks_processed = 0usize;

    let iter = scanner::walk(folder)?;
    for entry in iter {
        let file = match entry {
            Ok(f) => f,
            Err(e) => {
                tracing::info!("extract skip: {e:#}");
                continue;
            }
        };
        let display = relative_display(&file.path, folder);
        let chunks = chunker::chunk(&file.contents, chunker::DEFAULT_CHUNK_BYTES);
        for (i, chunk) in chunks.iter().enumerate() {
            let source_hint = format!("{display}:{i}");
            tracing::info!("extract: {source_hint} ({} bytes)", chunk.len());
            eprintln!("  extract: {source_hint} ({} bytes)", chunk.len());
            match client.extract(chunk, &source_hint).await {
                Ok(terms) => {
                    merger::merge_into(&mut db, terms);
                }
                Err(e) => {
                    tracing::warn!("extract failure on {source_hint}: {e:#}");
                }
            }
            chunks_processed += 1;
        }
    }
    merger::finalize(&mut db);
    crate::terms::save(&db, out_path)
        .with_context(|| format!("write {}", out_path.display()))?;
    tracing::info!(
        "extract done: {} chunks, {} unique terms → {}",
        chunks_processed,
        db.entries.len(),
        out_path.display()
    );
    Ok(chunks_processed)
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
