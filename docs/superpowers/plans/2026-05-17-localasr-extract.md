# localASR `extract` Subcommand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `localasr extract <folder> [--out <path>]` — a subcommand that scans a folder for text files, sends ~8KB chunks to the configured editor endpoint with a glossary-extraction prompt, and writes a strictly-formatted `terms.toml` that the daemon's heavy editor pass can later use as a domain dictionary.

**Architecture:** A four-stage pipeline in `src/extract/`:
1. **Scanner** (`scanner.rs`) — walks the folder via the `ignore` crate (respects `.gitignore` + hidden + standard ignores), filters out binaries and oversized files, yields `(PathBuf, String)` pairs.
2. **Chunker** (`chunker.rs`) — splits each file's contents into ≤8KB windows on line boundaries (lines that themselves exceed 8KB are emitted as their own chunk).
3. **Extractor** (`extractor.rs`) — for each chunk, posts a chat-completion request to the configured editor endpoint with a JSON-output system prompt; parses the response into `Vec<Term>`.
4. **Merger** (`merger.rs`) — accumulates terms across all chunks, dedupes by `name` (union of aliases, first non-empty hint wins), sorts alphabetically.

The orchestrator (`mod.rs::run`) glues them together sequentially (concurrency = 1, per spec, to avoid rate-limit complexity). Output is written via a new `terms::save()` helper.

**Tech Stack:** existing (`reqwest`, `serde`, `anyhow`, `tokio`) + one new dep `ignore = "0.4"` for `.gitignore`-respecting directory walking.

**Spec:** `docs/superpowers/specs/2026-05-17-localasr-design.md` — the "Subcommands" section bullet on `extract` + the `terms.toml` schema.

**Prerequisites:** Plans 1, 2, 3 complete. This plan starts a NEW branch `feat/extract` from the current `feat/wizard` HEAD (`95b9396`).

---

## File Structure

| File | Responsibility |
|---|---|
| `src/extract/mod.rs` | Public entry: `pub async fn run(folder, out, cfg) -> Result<()>`; re-exports submodules |
| `src/extract/scanner.rs` | `walk(root) -> impl Iterator<Item = Result<(PathBuf, String)>>` |
| `src/extract/chunker.rs` | `chunk(text, max_bytes: usize) -> Vec<String>` |
| `src/extract/extractor.rs` | `ExtractClient::new(cfg)` + `extract(chunk, source_hint) -> Result<Vec<Term>>` |
| `src/extract/merger.rs` | `merge_into(db: &mut TermDb, new: Vec<Term>)` + `finalize(db: &mut TermDb)` (sort + dedup) |
| `src/terms/mod.rs` | Add `pub fn save(db: &TermDb, path: &Path) -> Result<()>` |
| `src/main.rs` | Replace the `Cmd::Extract { .. }` `bail!` with a real dispatch into `extract::run` |
| `src/lib.rs` | Add `pub mod extract;` |
| `Cargo.toml` | Add `ignore = "0.4"` to `[dependencies]` |
| `README.md` | Add "Term-database extraction" section |

---

## Build Sequence Rationale

Bottom-up: scanner, chunker, merger, save — all pure(-ish) functions that can be unit-tested without HTTP or filesystem ceremony. Then the HTTP extractor (wiremock-tested). Then the orchestrator (integration-tested end-to-end with `wiremock` + `tempfile`). Finally CLI + README.

This order means the orchestrator can be written once all its dependencies are solid, with no rework.

The hardest single piece is the HTTP extractor (Task 5): the prompt has to be specific enough to get JSON back reliably from a typical LLM, the parser has to be lenient enough to handle minor formatting deviations, and the trait shape has to be testable without burning real API budget.

---

### Task 1: Branch + scanner with .gitignore support

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Create: `src/extract/mod.rs`
- Create: `src/extract/scanner.rs`

- [ ] **Step 1: Branch from current `feat/wizard` HEAD**

```bash
cd /home/kciceblue/HF/localASR
git checkout feat/wizard
git checkout -b feat/extract
```

(Confirm `git branch --show-current` says `feat/extract`. If the branch already exists, skip the `-b` and check out into a clean state.)

- [ ] **Step 2: Add `ignore` to `Cargo.toml`**

In `[dependencies]`, add:

```toml
ignore = "0.4"
```

This is BurntSushi's `ignore` crate (used by ripgrep). It walks directories respecting `.gitignore`, `.ignore`, hidden-file rules, and global excludes. Pure Rust, no system deps.

Run `cargo build` after the edit; it'll pull the new dep but otherwise be a no-op.

- [ ] **Step 3: Create `src/extract/mod.rs` (placeholder)**

```rust
//! `localasr extract <folder>` — generate a domain-specific terms database
//! by sending file chunks through the configured editor endpoint with a
//! glossary-extraction prompt.

pub mod scanner;
// chunker, extractor, merger, run() land in later tasks.
```

- [ ] **Step 4: Wire `pub mod extract;` into `src/lib.rs`**

Maintain alphabetical order. After the change:

```rust
pub mod config;
pub mod daemon;
pub mod doctor;
pub mod extract;
pub mod platform;
pub mod terms;
pub mod wizard;
```

- [ ] **Step 5: Write `src/extract/scanner.rs`**

```rust
//! Walks a folder yielding (path, contents) for files that look like text
//! and aren't ignored by .gitignore / hidden rules.
//!
//! Filters applied:
//! - `ignore` crate walker excludes hidden files, .gitignore-listed paths,
//!   and the user's global ignore rules.
//! - Binary detection: read the first 4 KB and decode as UTF-8; if that
//!   fails, skip the file.
//! - Size cap: files larger than `MAX_FILE_BYTES` are skipped (likely
//!   autogenerated or not source).

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

pub const MAX_FILE_BYTES: u64 = 256 * 1024; // 256 KB

pub struct ScannedFile {
    pub path: PathBuf,
    pub contents: String,
}

/// Walk `root` and return an iterator over scanned text files.
/// Errors during traversal are propagated as iterator items; the caller
/// can choose to log-and-skip vs. abort.
pub fn walk(root: &Path) -> Result<impl Iterator<Item = Result<ScannedFile>>> {
    if !root.exists() {
        anyhow::bail!("not found: {}", root.display());
    }
    if !root.is_dir() {
        anyhow::bail!("not a directory: {}", root.display());
    }
    let walker = WalkBuilder::new(root)
        .hidden(true)        // skip hidden files (default true, explicit for clarity)
        .git_ignore(true)    // honour .gitignore
        .git_global(true)    // honour user's global ignore
        .git_exclude(true)   // honour .git/info/exclude
        .build();

    Ok(walker.filter_map(|entry| {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Some(Err(anyhow::anyhow!("walk error: {e}"))),
        };
        // Skip directories themselves; we only yield files.
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            return None;
        }
        let path = entry.path().to_path_buf();
        Some(read_text_file(path))
    }))
}

fn read_text_file(path: PathBuf) -> Result<ScannedFile> {
    let meta = std::fs::metadata(&path)
        .with_context(|| format!("stat {}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        anyhow::bail!(
            "skipped (over {} bytes): {}",
            MAX_FILE_BYTES,
            path.display()
        );
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read {}", path.display()))?;
    if !looks_like_text(&bytes) {
        anyhow::bail!("skipped (binary): {}", path.display());
    }
    let contents = String::from_utf8(bytes)
        .with_context(|| format!("decode utf-8 from {}", path.display()))?;
    Ok(ScannedFile { path, contents })
}

/// Heuristic: a file is "text" if the first 4 KB is valid UTF-8 and
/// contains no null bytes. This matches what most diff/grep tools do.
fn looks_like_text(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(4096)];
    if probe.contains(&0) {
        return false;
    }
    std::str::from_utf8(probe).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, contents).unwrap();
        p
    }

    #[test]
    fn looks_like_text_accepts_utf8() {
        assert!(looks_like_text(b"hello world\n"));
        assert!(looks_like_text("café".as_bytes()));
    }

    #[test]
    fn looks_like_text_rejects_null_bytes() {
        assert!(!looks_like_text(b"hello\x00world"));
    }

    #[test]
    fn walk_yields_plain_files() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "a.txt", b"hello\n");
        write(tmp.path(), "b.rs", b"fn main() {}\n");
        let mut names: Vec<String> = walk(tmp.path())
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.rs"]);
    }

    #[test]
    fn walk_respects_gitignore() {
        let tmp = TempDir::new().unwrap();
        // The `ignore` crate honours .gitignore only inside a git repo OR
        // when the root has a .gitignore directly. Both work here.
        write(tmp.path(), ".gitignore", b"ignored.txt\n");
        write(tmp.path(), "kept.txt", b"keep me\n");
        write(tmp.path(), "ignored.txt", b"skip me\n");
        let names: Vec<String> = walk(tmp.path())
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .filter(|n| n != ".gitignore")
            .collect();
        assert_eq!(names, vec!["kept.txt"]);
    }

    #[test]
    fn walk_skips_hidden() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), ".hidden", b"x");
        write(tmp.path(), "visible.txt", b"y");
        let names: Vec<String> = walk(tmp.path())
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["visible.txt"]);
    }

    #[test]
    fn walk_skips_binary() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "image.png", b"\x89PNG\x00binary stuff");
        write(tmp.path(), "code.rs", b"fn main() {}");
        let results: Vec<_> = walk(tmp.path()).unwrap().collect();
        // The binary file appears as Err(...) so the caller can log it.
        let oks: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        let errs: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
        assert_eq!(oks.len(), 1);
        assert_eq!(oks[0].path.file_name().unwrap(), "code.rs");
        assert!(errs.iter().any(|e| format!("{e:#}").contains("binary")));
    }

    #[test]
    fn walk_errors_for_missing_root() {
        let tmp = TempDir::new().unwrap();
        let bogus = tmp.path().join("does-not-exist");
        assert!(walk(&bogus).is_err());
    }

    #[test]
    fn walk_errors_for_file_root() {
        let tmp = TempDir::new().unwrap();
        let file = write(tmp.path(), "plain.txt", b"hi");
        assert!(walk(&file).is_err());
    }
}
```

- [ ] **Step 6: Run tests + build**

```bash
cargo build
cargo test --lib extract::scanner::
```

Expected: 7 passing tests; whole crate builds clean (the pre-existing `daemon::vad::Chunker.sample_rate` dead-code warning is unrelated).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(extract): scaffold + scanner with .gitignore-aware walk"
```

---

### Task 2: Chunker on line boundaries

**Files:**
- Create: `src/extract/chunker.rs`
- Modify: `src/extract/mod.rs` (`pub mod chunker;`)

- [ ] **Step 1: Write `src/extract/chunker.rs`**

```rust
//! Split a string into ≤max_bytes windows on line boundaries.
//!
//! Algorithm: accumulate lines into a buffer; whenever adding the next line
//! (including its trailing `\n`) would push the buffer past `max_bytes`,
//! emit the buffer as a chunk and start fresh. A single line that itself
//! exceeds `max_bytes` is emitted as its own chunk — we never split mid-line.

pub const DEFAULT_CHUNK_BYTES: usize = 8 * 1024;

pub fn chunk(text: &str, max_bytes: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in text.split_inclusive('\n') {
        // If the line itself is bigger than max_bytes:
        //   - flush whatever's accumulated (so order is preserved),
        //   - then emit the long line as its own chunk.
        if line.len() > max_bytes {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push(line.to_string());
            continue;
        }
        // If adding this line would exceed the cap, flush first.
        if !buf.is_empty() && buf.len() + line.len() > max_bytes {
            out.push(std::mem::take(&mut buf));
        }
        buf.push_str(line);
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() {
        assert!(chunk("", 8 * 1024).is_empty());
    }

    #[test]
    fn small_input_is_one_chunk() {
        let out = chunk("hello\nworld\n", 8 * 1024);
        assert_eq!(out, vec!["hello\nworld\n"]);
    }

    #[test]
    fn splits_on_line_boundary() {
        // Each line is 10 bytes ("0123456789\n" is 11). Cap at 22 bytes per
        // chunk: we should get pairs of lines.
        let text: String = (0..6)
            .map(|i| format!("line-{i:03}\n")) // 10 bytes each
            .collect();
        let out = chunk(&text, 22);
        // 22 / 10 = 2.2 lines per chunk → 2 lines, since adding a 3rd would exceed.
        assert!(out.iter().all(|c| c.len() <= 22), "all chunks within cap");
        // Concatenation preserves the input.
        assert_eq!(out.concat(), text);
    }

    #[test]
    fn never_splits_mid_line() {
        let big_line = "a".repeat(20_000) + "\n";
        let text = format!("short\n{big_line}also-short\n");
        let out = chunk(&text, 8 * 1024);
        // The big line is emitted whole (chunk length > cap is allowed
        // specifically for over-sized single lines).
        assert!(out.iter().any(|c| c.len() > 8 * 1024));
        // No chunk is the big line concatenated with something else.
        assert!(out.iter().all(|c| c == &big_line || !c.contains(&big_line[..])));
        // Total preserved.
        assert_eq!(out.concat(), text);
    }

    #[test]
    fn trailing_text_without_newline_is_emitted() {
        let out = chunk("no-newline-at-end", 8 * 1024);
        assert_eq!(out, vec!["no-newline-at-end"]);
    }
}
```

- [ ] **Step 2: Add `pub mod chunker;` to `src/extract/mod.rs`**

```rust
pub mod chunker;
pub mod scanner;
// extractor, merger, run() in later tasks.
```

- [ ] **Step 3: Run tests + build**

```bash
cargo test --lib extract::chunker::
```

Expected: 5 passing tests.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(extract): line-bounded chunker"
```

---

### Task 3: Term merger (dedup + sort + alias union)

**Files:**
- Create: `src/extract/merger.rs`
- Modify: `src/extract/mod.rs` (`pub mod merger;`)

- [ ] **Step 1: Write `src/extract/merger.rs`**

```rust
//! Merge term-extraction results from multiple chunks into a single TermDb.
//!
//! Semantics:
//! - Dedup by `name` (case-sensitive — `kubectl` and `Kubectl` are distinct).
//! - On collision: union aliases (preserving first-seen order, dedup),
//!   keep the FIRST non-empty hint.
//! - `finalize()` sorts entries alphabetically by `name` for stable output.

use crate::terms::{Term, TermDb};
use std::collections::HashMap;

/// Merge `new` terms into `db`. Duplicates are folded per the module-level rules.
pub fn merge_into(db: &mut TermDb, new: Vec<Term>) {
    // Build an index of existing names → position in db.entries.
    let mut index: HashMap<String, usize> = HashMap::with_capacity(db.entries.len());
    for (i, t) in db.entries.iter().enumerate() {
        index.insert(t.name.clone(), i);
    }
    for incoming in new {
        if incoming.name.is_empty() {
            continue;
        }
        match index.get(&incoming.name).copied() {
            None => {
                index.insert(incoming.name.clone(), db.entries.len());
                db.entries.push(incoming);
            }
            Some(pos) => {
                let existing = &mut db.entries[pos];
                // Union aliases, preserving first-seen order.
                for a in incoming.aliases {
                    if !existing.aliases.iter().any(|x| x == &a) {
                        existing.aliases.push(a);
                    }
                }
                // Keep first non-empty hint.
                if existing.hint.is_empty() && !incoming.hint.is_empty() {
                    existing.hint = incoming.hint;
                }
            }
        }
    }
}

/// Sort the db's entries alphabetically by name. Call once after all
/// `merge_into()` calls have completed.
pub fn finalize(db: &mut TermDb) {
    db.entries.sort_by(|a, b| a.name.cmp(&b.name));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, aliases: &[&str], hint: &str) -> Term {
        Term {
            name: name.into(),
            aliases: aliases.iter().map(|s| (*s).into()).collect(),
            hint: hint.into(),
        }
    }

    #[test]
    fn merge_into_empty_db_adds_all() {
        let mut db = TermDb { entries: vec![] };
        merge_into(&mut db, vec![t("a", &["x"], "h-a"), t("b", &[], "")]);
        assert_eq!(db.entries.len(), 2);
        assert_eq!(db.entries[0].name, "a");
        assert_eq!(db.entries[0].aliases, vec!["x"]);
    }

    #[test]
    fn merge_into_dedups_by_name() {
        let mut db = TermDb { entries: vec![t("k", &["a"], "first")] };
        merge_into(&mut db, vec![t("k", &["b"], "second")]);
        assert_eq!(db.entries.len(), 1);
        assert_eq!(db.entries[0].aliases, vec!["a", "b"]);
        // First non-empty hint wins.
        assert_eq!(db.entries[0].hint, "first");
    }

    #[test]
    fn merge_into_fills_empty_hint() {
        let mut db = TermDb { entries: vec![t("k", &[], "")] };
        merge_into(&mut db, vec![t("k", &[], "now-i-know")]);
        assert_eq!(db.entries[0].hint, "now-i-know");
    }

    #[test]
    fn merge_into_unions_aliases_no_dup() {
        let mut db = TermDb { entries: vec![t("k", &["a", "b"], "")] };
        merge_into(&mut db, vec![t("k", &["b", "c"], "")]);
        assert_eq!(db.entries[0].aliases, vec!["a", "b", "c"]);
    }

    #[test]
    fn merge_into_skips_empty_name() {
        let mut db = TermDb { entries: vec![] };
        merge_into(&mut db, vec![t("", &["x"], "h")]);
        assert!(db.entries.is_empty());
    }

    #[test]
    fn finalize_sorts_by_name() {
        let mut db = TermDb {
            entries: vec![t("kubectl", &[], ""), t("ansible", &[], ""), t("tokio", &[], "")],
        };
        finalize(&mut db);
        let names: Vec<&str> = db.entries.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["ansible", "kubectl", "tokio"]);
    }
}
```

- [ ] **Step 2: Add `pub mod merger;` to `src/extract/mod.rs`**

- [ ] **Step 3: Run tests**

```bash
cargo test --lib extract::merger::
```

Expected: 6 passing tests.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(extract): term-db merger with alias union + alpha sort"
```

---

### Task 4: `terms::save()`

**Files:**
- Modify: `src/terms/mod.rs` (add `save` function + tests)

- [ ] **Step 1: Append to `src/terms/mod.rs`**

After the existing `load()` and `render_prompt_block()` definitions, add:

```rust
/// Serialize `db` as TOML and write it atomically to `path`.
/// Creates parent directories as needed.
pub fn save(db: &TermDb, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(db)
        .context("serialize TermDb to TOML")?;
    std::fs::write(path, text)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
```

Then add tests inside the existing `#[cfg(test)] mod tests { ... }` block (don't create a new mod):

```rust
    #[test]
    fn save_then_load_roundtrips() {
        let db = TermDb {
            entries: vec![
                Term { name: "kubectl".into(), aliases: vec!["cube control".into()], hint: "k8s cli".into() },
                Term { name: "tokio".into(), aliases: vec![], hint: "".into() },
            ],
        };
        let tmp = tempfile::NamedTempFile::new().unwrap();
        save(&db, tmp.path()).unwrap();
        let loaded = load(tmp.path()).unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].name, "kubectl");
        assert_eq!(loaded.entries[1].aliases, Vec::<String>::new());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("a").join("b").join("terms.toml");
        let db = TermDb { entries: vec![] };
        save(&db, &nested).unwrap();
        assert!(nested.exists());
    }
```

(The `tempfile` crate is already in dev-dependencies.)

- [ ] **Step 2: Run tests**

```bash
cargo test --lib terms::tests
```

Expected: prior terms tests (3 or 4) + 2 new = pass.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(terms): save() helper writes TermDb as pretty TOML"
```

---

### Task 5: LLM extractor client (HTTP + JSON parse)

**Files:**
- Create: `src/extract/extractor.rs`
- Modify: `src/extract/mod.rs` (`pub mod extractor;`)
- Modify: `Cargo.toml` if `wiremock` is missing (it's already in `[dev-dependencies]` — verify).

- [ ] **Step 1: Write `src/extract/extractor.rs`**

```rust
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
            tracing::warn!("extract: JSON parse failed: {e} | raw start: {:?}", &cleaned.chars().take(80).collect::<String>());
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
```

- [ ] **Step 2: Add `pub mod extractor;` to `src/extract/mod.rs`**

- [ ] **Step 3: Run tests + build**

```bash
cargo test --lib extract::extractor::
```

Expected: 6 sync + 3 async tests = 9 passing.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(extract): chat-completion extractor with JSON parsing"
```

---

### Task 6: Orchestrator (`extract::run`)

**Files:**
- Modify: `src/extract/mod.rs` (add `run`)
- Create: `tests/extract.rs` (integration test with wiremock + tempdir)

- [ ] **Step 1: Replace `src/extract/mod.rs` with the orchestrator**

Keep all `pub mod` declarations; add the `run()` entry below them.

```rust
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
use std::path::{Path, PathBuf};

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
                // Skipped (binary / oversized / unreadable). Log and continue.
                tracing::info!("extract skip: {e:#}");
                continue;
            }
        };
        let display = relative_display(&file.path, folder);
        let chunks = chunker::chunk(&file.contents, chunker::DEFAULT_CHUNK_BYTES);
        for (i, chunk) in chunks.iter().enumerate() {
            let source_hint = format!("{display}#{i}");
            tracing::info!("extract: {source_hint} ({} bytes)", chunk.len());
            match client.extract(chunk, &source_hint).await {
                Ok(terms) => {
                    merger::merge_into(&mut db, terms);
                }
                Err(e) => {
                    // Per-chunk failure: log and skip; don't abort the run.
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
```

- [ ] **Step 2: Write `tests/extract.rs`**

```rust
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
    // The mock returns the same response for every chunk: 2 terms.
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

    // Re-load via the terms loader and assert dedup + sort.
    let db = localasr::terms::load(&out).unwrap();
    let names: Vec<&str> = db.entries.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["kubectl", "tokio"]); // sorted, deduped across 2+ chunks
    let kubectl = &db.entries[0];
    assert_eq!(kubectl.aliases, vec!["cube control"]);
    assert_eq!(kubectl.hint, "k8s cli");
}

#[tokio::test]
async fn extract_silently_skips_per_chunk_failures() {
    let server = MockServer::start().await;
    // Server returns 500 for every chunk; extract should still write an
    // (empty) terms.toml and not bail.
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
    // Server never gets called.
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
    // Don't even need a server — should bail before any HTTP.
    let out = PathBuf::from("/tmp/this-doesnt-matter.toml");
    let cfg_dummy = cfg("http://127.0.0.1:9");
    let err = localasr::extract::run(std::path::Path::new("/nonexistent-9d8a2"), &out, cfg_dummy)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"));
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib
cargo test --test extract
```

Expected: lib tests still pass (unchanged count from prior task — Task 6 adds no lib tests, just orchestrator code); integration test adds 4 passing (full `cargo test` total = `cargo test --lib` total + 1 existing e2e + 4 new extract = previous total + 4).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(extract): orchestrator + end-to-end integration tests"
```

---

### Task 7: CLI wiring + README

**Files:**
- Modify: `src/main.rs`
- Modify: `README.md`

- [ ] **Step 1: Add an `--out` CLI flag to `Cmd::Extract`**

Currently in `src/main.rs` the variant is:

```rust
Extract { folder: String },
```

Change to:

```rust
Extract {
    /// Folder to scan (recursive, respects .gitignore).
    folder: String,
    /// Output path for terms.toml. Defaults to `[terms].path` from config.
    #[arg(long)]
    out: Option<String>,
},
```

And replace the `Cmd::Extract { .. }` arm in `main()`:

```rust
Cmd::Extract { folder, out } => run_extract(folder, out).await,
```

Add the dispatcher function in `src/main.rs`:

```rust
async fn run_extract(folder: String, out: Option<String>) -> anyhow::Result<()> {
    let folder = std::path::PathBuf::from(folder);

    // Resolve output path: --out flag wins; else config's [terms].path.
    let out_path = match out {
        Some(p) => std::path::PathBuf::from(shellexpand::tilde(&p).to_string()),
        None => {
            let cfg_path = localasr::config::default_config_path()?;
            if !cfg_path.exists() {
                anyhow::bail!(
                    "No --out given and no config at {} to read [terms].path from. \
                     Run `localasr setup` or pass --out <path>.",
                    cfg_path.display()
                );
            }
            let cfg = localasr::config::load(&cfg_path)?;
            std::path::PathBuf::from(shellexpand::tilde(&cfg.terms.path).to_string())
        }
    };

    // Load editor config (always required — extract uses the editor endpoint).
    let cfg_path = localasr::config::default_config_path()?;
    if !cfg_path.exists() {
        anyhow::bail!(
            "No config at {}. Extract needs the [editor] section. Run `localasr setup`.",
            cfg_path.display()
        );
    }
    let cfg = localasr::config::load(&cfg_path)?;

    let chunks = localasr::extract::run(&folder, &out_path, cfg.editor).await?;
    println!(
        "extract: {} chunks processed → {}",
        chunks,
        out_path.display()
    );
    Ok(())
}
```

(The `shellexpand::tilde` import is already used elsewhere in the project; verify it's in scope here. If not, `use shellexpand;` at the top of main.rs — or qualify as we did above.)

- [ ] **Step 2: Update `README.md`**

Add a "## Term-database extraction" section just after the "## Setup wizard" section. Suggested content:

```markdown
## Term-database extraction

To help the heavy editor pass disambiguate jargon and names, you can
auto-generate a `terms.toml` from an existing corpus (e.g. your own
codebase or documentation folder):

```
localasr extract path/to/folder
```

This scans the folder (respecting `.gitignore`), splits each text file into
~8KB chunks, asks the configured editor endpoint to identify domain-specific
terms, and writes a `terms.toml` to the path in your `[terms]` config
section (default `~/.config/localasr/terms.toml`).

Pass `--out <path>` to override the destination.

The result is merged and deduplicated; entries look like:

```toml
[[terms]]
name = "kubectl"
aliases = ["cube control", "cube cuttle"]
hint = "Kubernetes CLI"
```

Enable it in your config:

```toml
[terms]
enabled = true
path = "~/.config/localasr/terms.toml"
```

Extraction runs sequentially against the editor endpoint, so a large folder
may take a while. Per-chunk failures are logged but don't abort the run.
```

Adapt the heading level / tone to match the rest of the README.

- [ ] **Step 3: Validate**

```bash
cargo build
cargo test
```

Expected: build clean; total test count = prior total (after Plan 3 polish = 78) + 6 new extract unit tests (Tasks 1+2+3+5: 7+5+6+9 = 27 lib tests) + 2 new terms save tests + 4 integration tests + 0 main.rs tests = 78 + 27 + 2 + 4 = **111 total**. Adjust if intermediate task numbers differ from your run; the key is "all pass".

- [ ] **Step 4: Manual smoke test (optional)**

If a llama.cpp / OpenAI-compatible server is reachable, run:

```bash
cargo run -- extract ./src --out /tmp/test-terms.toml
```

and inspect the output. (Don't add this as an automated test — it requires a real LLM.)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(extract): wire \`localasr extract\` into CLI + README"
```

---

## Self-Review

(Performed by the plan author after writing.)

**Spec coverage:**
- *"scan a folder"* — Task 1 (scanner).
- *".gitignore respected"* — Task 1 (scanner uses `ignore::WalkBuilder` with `git_ignore(true)`).
- *"chunked into ~8KB windows on line boundaries"* — Task 2 (chunker with `DEFAULT_CHUNK_BYTES = 8 * 1024`).
- *"send … to the editor endpoint with a glossary-extraction prompt"* — Task 5 (extractor; reuses `[editor]` config block).
- *"sequentially (concurrency = 1)"* — Task 6 (orchestrator's `for await` loop).
- *"merged, deduped by `name`, and sorted alphabetically"* — Task 3 (merger).
- *"write strictly-formatted `terms.toml`"* — Task 4 (`terms::save`) + the existing strict schema in `src/terms/schema.rs`.
- *"default path from config"* + `--out <path>` — Task 7 (CLI).

**Placeholder scan:** No TBD/TODO. Every code block is complete. The `ignore` crate version is pinned `0.4`; if a newer major exists, the engineer should use the latest 0.x and adapt API calls (the `WalkBuilder` shape is stable across 0.4.x).

**Type consistency:**
- `Term { name, aliases, hint }` matches `src/terms/schema.rs` exactly.
- `TermDb { entries }` ditto (note: serde renames `entries` ↔ `terms` in the TOML, so the in-memory field is `entries`).
- `EditorConfig` signature matches `src/config/schema.rs`.
- `extract::run` returns `Result<usize>`; CLI prints the chunks count.
- `ScannedFile { path, contents }` is consistent across tasks.

**Scope check:** Single coherent feature (one CLI subcommand). 7 tasks. The largest single piece is the extractor (Task 5) because of the prompt + parsing + HTTP plumbing; the rest are mechanical. Integration test in Task 6 ties everything together without needing real LLM access.

**Known limitations:**
- Sequential extraction is slow for big repos. v1 accepts this per spec.
- No retry on per-chunk HTTP failure (just log and skip). v1 accepts this; an interactive `extract --retry` is deferred.
- The system prompt is hand-tuned but the response quality depends on the model. Garbage responses yield zero terms (logged WARN); the user sees the chunk count vs. resulting entry count and can re-run with a better model.
- No progress bar. `tracing::info!` lines are the only visible progress; users can crank `RUST_LOG=info` to see them.
- The chunker uses naive `.len()` (byte length) for size limits, not character count. Multi-byte UTF-8 sequences are split between chunks only on `\n`, so this is safe.
- Files larger than 256 KB are skipped wholesale. Real auto-generated source is often bigger; tuning the cap is left to a future flag.
