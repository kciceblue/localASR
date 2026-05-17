# localASR Core Daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the core push-to-talk daemon: hotkey → audio → VAD-chunked ASR → light editor (per pause) → heavy editor (on release, optional term DB) → clipboard-paste injection with backspace+retype corrections. Cross-platform (Linux + Windows). No GUI in this plan; users hand-write `config.toml` until the wizard ships in Plan 3.

**Architecture:** Single Rust crate. Tokio orchestrates async HTTP and the pipeline. Two dedicated OS threads: one for `cpal` audio capture, one for blocking hotkey reads. Dependency injection throughout (traits for `Platform`, `Asr`, `Editor`, `Vad`) so the pipeline is fully testable with mocks + `wiremock`. Platform-specific code lives behind a single `Platform` trait with `linux/` and `windows/` implementations selected via `#[cfg(...)]`.

**Tech Stack:** Rust 2021, Tokio, `clap`, `serde`+`toml`, `reqwest` (HTTP + multipart), `cpal` (audio), `evdev` (Linux hotkey + uinput), `arboard` (clipboard), `voice_activity_detector` (silero VAD) + `webrtc-vad`, `windows` crate (Windows), `wiremock` (HTTP test mocks), `tracing` (logging).

**Spec:** `docs/superpowers/specs/2026-05-17-localasr-design.md`

**Out of scope for this plan (later plans cover):** `localasr setup` wizard, `localasr doctor` subcommand, `localasr extract` subcommand.

---

## File Structure

Files created by this plan (all under `src/` unless noted):

| File | Responsibility |
|---|---|
| `Cargo.toml` | Workspace deps, target-gated platform crates |
| `.gitignore` | Rust + IDE noise |
| `README.md` | Build + hand-written config instructions for early users |
| `src/main.rs` | clap dispatch; routes to daemon subcommand |
| `src/lib.rs` | Re-exports for integration tests |
| `src/config/mod.rs` | Path resolution, load, `${ENV}` interpolation |
| `src/config/schema.rs` | Strict serde types for `config.toml` |
| `src/terms/mod.rs` | Load `terms.toml`, render to prompt block |
| `src/terms/schema.rs` | Strict serde types for `terms.toml` |
| `src/daemon/mod.rs` | Daemon lifecycle, session manager, abort-on-re-press |
| `src/daemon/state.rs` | Typed-state tracker, diff → (backspace_count, new_tail) |
| `src/daemon/vad.rs` | `Vad` trait + silero and webrtc backends + chunking state machine |
| `src/daemon/audio.rs` | `cpal` capture thread → crossbeam channel of i16 frames |
| `src/daemon/asr_client.rs` | OpenAI `/v1/audio/transcriptions` client |
| `src/daemon/editor_client.rs` | OpenAI `/v1/chat/completions` client (light + heavy) |
| `src/daemon/injector.rs` | Clipboard save/restore + paste + backspace |
| `src/daemon/pipeline.rs` | Per-session async orchestrator |
| `src/daemon/hotkey.rs` | Platform-dispatched hotkey listener |
| `src/platform/mod.rs` | `Platform` trait + `MockPlatform` |
| `src/platform/linux/mod.rs` | Linux `Platform` impl, runtime composition |
| `src/platform/linux/hotkey.rs` | evdev key listener |
| `src/platform/linux/input.rs` | uinput virtual keyboard for paste/backspace |
| `src/platform/linux/clipboard.rs` | `arboard` wrapper |
| `src/platform/windows/mod.rs` | Windows `Platform` impl |
| `src/platform/windows/hotkey.rs` | `SetWindowsHookExW(WH_KEYBOARD_LL)` |
| `src/platform/windows/input.rs` | `SendInput` |
| `src/platform/windows/clipboard.rs` | OpenClipboard/Get/Set |
| `tests/end_to_end.rs` | Pipeline integration test with mocks |
| `99-localasr.rules` (repo root) | Sample udev rule for `/dev/uinput` access |

---

## Build Sequence Rationale

Tasks land bottom-up: pure data types first, then HTTP clients, then the platform trait, then the orchestrator, then the daemon shell. The pipeline (Task 14) needs everything below it to exist as injectable trait impls — but every dependency has a mock equivalent, so the pipeline's tests don't need any hardware.

Tasks 1–8 are platform-independent and fully testable.
Task 9 introduces the platform trait + MockPlatform.
Tasks 10–12 add Linux platform code (must run `cargo check` on Linux).
Task 13 adds Windows platform code (must run `cargo check --target x86_64-pc-windows-gnu` to verify it compiles; runtime testing is manual on Windows).
Tasks 14–16 wire everything together.

---

### Task 1: Project scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `README.md`
- Create: `src/main.rs`
- Create: `src/lib.rs`

- [ ] **Step 1: Initialize git and write `.gitignore`**

```bash
cd /home/kciceblue/HF/localASR
git init
```

Then write `.gitignore`:

```gitignore
/target
**/*.rs.bk
*.pdb
.idea/
.vscode/
*.swp
*.swo
config.toml.local
```

- [ ] **Step 2: Write `Cargo.toml`**

```toml
[package]
name = "localasr"
version = "0.1.0"
edition = "2021"
description = "Cross-platform push-to-talk dictation with OpenAI-compatible ASR + editor LLM"
license = "MIT"

[lib]
name = "localasr"
path = "src/lib.rs"

[[bin]]
name = "localasr"
path = "src/main.rs"

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "fs", "signal"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
reqwest = { version = "0.12", features = ["json", "multipart", "stream"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
crossbeam-channel = "0.5"
cpal = "0.15"
arboard = "3"
anyhow = "1"
thiserror = "2"
directories = "5"
shellexpand = "3"
async-trait = "0.1"
bytes = "1"
voice_activity_detector = "0.2"
webrtc-vad = "0.4"
hound = "3"  # WAV encoding for ASR upload

[target.'cfg(target_os = "linux")'.dependencies]
evdev = { version = "0.13", features = ["tokio"] }

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_WindowsAndMessaging",
    "Win32_System_DataExchange",
    "Win32_System_Memory",
    "Win32_System_Threading",
    "Win32_System_LibraryLoader",
] }

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
assert_matches = "1"
```

- [ ] **Step 3: Write minimal `src/main.rs`**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "localasr", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon (default if no subcommand given)
    Daemon,
    /// (placeholder) GUI setup wizard — see Plan 3
    Setup,
    /// (placeholder) Diagnostics — see Plan 2
    Doctor,
    /// (placeholder) Term database extractor — see Plan 4
    Extract { folder: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Cmd::Daemon) {
        Cmd::Daemon => {
            tracing::info!("daemon: not yet implemented");
            Ok(())
        }
        Cmd::Setup => {
            anyhow::bail!("`localasr setup` lands in Plan 3");
        }
        Cmd::Doctor => {
            anyhow::bail!("`localasr doctor` lands in Plan 2");
        }
        Cmd::Extract { .. } => {
            anyhow::bail!("`localasr extract` lands in Plan 4");
        }
    }
}
```

- [ ] **Step 4: Write minimal `src/lib.rs`**

```rust
// Re-exports for integration tests under `tests/`.
// Modules are added as later tasks land.
```

- [ ] **Step 5: Write minimal `README.md`**

```markdown
# localASR

Cross-platform push-to-talk dictation using OpenAI-compatible ASR + editor endpoints.

Status: under construction. See `docs/superpowers/specs/` and `docs/superpowers/plans/`.

## Build

```
cargo build --release
```

## Linux setup (developer mode)

Install the sample udev rule so the daemon can synthesize input:

```
sudo cp 99-localasr.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
```

Then add yourself to the `input` group (log out and back in to apply):

```
sudo usermod -aG input $USER
```
```

- [ ] **Step 6: Verify it compiles and runs**

```bash
cargo build
cargo run -- --version
cargo run -- daemon
```

Expected: prints version, then `daemon: not yet implemented`, exits 0.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: project scaffolding with clap subcommand skeleton"
```

---

### Task 2: Config schema and loader

**Files:**
- Create: `src/config/mod.rs`
- Create: `src/config/schema.rs`
- Modify: `src/lib.rs` (add `pub mod config;`)

- [ ] **Step 1: Write failing tests in `src/config/mod.rs`**

```rust
pub mod schema;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Resolve the default config path per platform (XDG on Linux, AppData on Windows).
pub fn default_config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "localasr")
        .context("could not resolve user config directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Load + parse + interpolate ${ENV} placeholders.
pub fn load(path: &Path) -> Result<schema::Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    let interpolated = interpolate_env(&raw)?;
    let cfg: schema::Config = toml::from_str(&interpolated)
        .with_context(|| format!("parsing TOML at {}", path.display()))?;
    Ok(cfg)
}

/// Replaces ${NAME} with the value of env var NAME.
/// `\$` is an escape for a literal `$`.
/// Missing env vars are an error.
pub fn interpolate_env(s: &str) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if c == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = s[i + 2..].find('}') {
                let name = &s[i + 2..i + 2 + end];
                let val = std::env::var(name)
                    .with_context(|| format!("env var ${{{name}}} not set"))?;
                out.push_str(&val);
                i = i + 2 + end + 1;
                continue;
            }
        }
        out.push(c as char);
        i += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_single_var() {
        std::env::set_var("LOCALASR_TEST_KEY", "abc123");
        assert_eq!(interpolate_env("k=${LOCALASR_TEST_KEY}").unwrap(), "k=abc123");
    }

    #[test]
    fn interpolates_multiple_vars() {
        std::env::set_var("LOCALASR_TEST_A", "one");
        std::env::set_var("LOCALASR_TEST_B", "two");
        assert_eq!(
            interpolate_env("${LOCALASR_TEST_A}-${LOCALASR_TEST_B}").unwrap(),
            "one-two"
        );
    }

    #[test]
    fn escaped_dollar_passes_through() {
        assert_eq!(interpolate_env(r"price=\$5").unwrap(), "price=$5");
    }

    #[test]
    fn missing_var_errors() {
        let err = interpolate_env("${LOCALASR_DEFINITELY_NOT_SET_XYZ}").unwrap_err();
        assert!(err.to_string().contains("not set"));
    }

    #[test]
    fn literal_dollar_without_brace_passes() {
        assert_eq!(interpolate_env("$100").unwrap(), "$100");
    }

    #[test]
    fn loads_minimal_valid_config() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), MINIMAL_CONFIG).unwrap();
        let cfg = load(tmp.path()).unwrap();
        assert_eq!(cfg.hotkey.binding, "RightCtrl");
        assert_eq!(cfg.asr.model, "whisper-1");
    }

    const MINIMAL_CONFIG: &str = r#"
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
path = "~/.config/localasr/terms.toml"

[injection]
mode = "clipboard_paste"
paste_shortcut = "Ctrl+V"
restore_delay_ms = 100
"#;
}
```

- [ ] **Step 2: Write `src/config/schema.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub vad: VadConfig,
    pub asr: AsrConfig,
    pub editor: EditorConfig,
    pub terms: TermsConfig,
    pub injection: InjectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyConfig {
    pub binding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioConfig {
    #[serde(default)]
    pub device: String,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VadConfig {
    pub backend: VadBackend,
    pub min_silence_ms: u32,
    pub max_chunk_ms: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VadBackend {
    Silero,
    Webrtc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub language: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub light_temperature: f32,
    pub heavy_temperature: f32,
    pub light_timeout_ms: u64,
    pub heavy_timeout_ms: u64,
    pub light: EditorPassConfig,
    pub heavy: EditorPassConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorPassConfig {
    pub enabled: bool,
    #[serde(default)]
    pub context_chunks: u32,
    #[serde(default)]
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermsConfig {
    pub enabled: bool,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InjectionConfig {
    pub mode: String,
    pub paste_shortcut: String,
    pub restore_delay_ms: u64,
}
```

- [ ] **Step 3: Wire up the module in `src/lib.rs`**

Replace contents:

```rust
pub mod config;
```

- [ ] **Step 4: Run tests; expect them to pass**

```bash
cargo test --lib config::
```

Expected: 6 passing tests.

- [ ] **Step 5: Add a test for rejecting unknown fields**

Append to `tests` module in `src/config/mod.rs`:

```rust
    #[test]
    fn rejects_unknown_field() {
        let bad = MINIMAL_CONFIG.to_string() + "\n[bogus]\nfoo = 1\n";
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bad).unwrap();
        let err = load(tmp.path()).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unknown") || err.to_string().contains("bogus"));
    }
```

- [ ] **Step 6: Run tests**

```bash
cargo test --lib config::
```

Expected: 7 passing tests.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(config): strict TOML schema + env-var interpolation"
```

---

### Task 3: Terms schema and loader

**Files:**
- Create: `src/terms/mod.rs`
- Create: `src/terms/schema.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write `src/terms/schema.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermDb {
    #[serde(rename = "terms")]
    pub entries: Vec<Term>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub name: String,
    pub aliases: Vec<String>,
    pub hint: String,
}
```

- [ ] **Step 2: Write `src/terms/mod.rs` with tests**

```rust
pub mod schema;

use anyhow::{Context, Result};
use std::path::Path;
pub use schema::{Term, TermDb};

pub fn load(path: &Path) -> Result<TermDb> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading terms at {}", path.display()))?;
    let db: TermDb = toml::from_str(&raw)
        .with_context(|| format!("parsing terms at {}", path.display()))?;
    Ok(db)
}

/// Render the term DB as lines suitable for a system prompt.
/// Each line: `name — hint (sounds-like: "alias1", "alias2")`.
/// Empty aliases / hint are omitted gracefully.
pub fn render_prompt_block(db: &TermDb) -> String {
    let mut out = String::new();
    for t in &db.entries {
        out.push_str(&t.name);
        if !t.hint.is_empty() {
            out.push_str(" \u{2014} ");
            out.push_str(&t.hint);
        }
        if !t.aliases.is_empty() {
            out.push_str(" (sounds-like: ");
            for (i, a) in t.aliases.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                out.push('"');
                out.push_str(a);
                out.push('"');
            }
            out.push(')');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[terms]]
name = "kubectl"
aliases = ["cube control"]
hint = "Kubernetes CLI tool"

[[terms]]
name = "tokio"
aliases = []
hint = ""
"#;

    #[test]
    fn loads_valid_terms() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), SAMPLE).unwrap();
        let db = load(tmp.path()).unwrap();
        assert_eq!(db.entries.len(), 2);
        assert_eq!(db.entries[0].name, "kubectl");
        assert_eq!(db.entries[1].aliases.len(), 0);
    }

    #[test]
    fn rejects_missing_required_field() {
        let bad = r#"
[[terms]]
name = "only-name"
"#;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bad).unwrap();
        assert!(load(tmp.path()).is_err());
    }

    #[test]
    fn rejects_unknown_field() {
        let bad = r#"
[[terms]]
name = "x"
aliases = []
hint = ""
extra = "nope"
"#;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bad).unwrap();
        assert!(load(tmp.path()).is_err());
    }

    #[test]
    fn prompt_block_includes_alias_and_hint() {
        let db = TermDb {
            entries: vec![Term {
                name: "kubectl".into(),
                aliases: vec!["cube control".into()],
                hint: "Kubernetes CLI tool".into(),
            }],
        };
        let s = render_prompt_block(&db);
        assert!(s.contains("kubectl"));
        assert!(s.contains("Kubernetes CLI tool"));
        assert!(s.contains("cube control"));
    }

    #[test]
    fn prompt_block_handles_empty_hint_and_aliases() {
        let db = TermDb {
            entries: vec![Term {
                name: "bare".into(),
                aliases: vec![],
                hint: "".into(),
            }],
        };
        let s = render_prompt_block(&db);
        assert_eq!(s.trim(), "bare");
    }
}
```

- [ ] **Step 3: Wire it up**

In `src/lib.rs`:

```rust
pub mod config;
pub mod terms;
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib terms::
```

Expected: 5 passing tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(terms): strict TOML schema + prompt-block renderer"
```

---

### Task 4: Typed-state tracker (diff state machine)

**Files:**
- Create: `src/daemon/mod.rs`
- Create: `src/daemon/state.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write `src/daemon/state.rs` with tests first**

```rust
/// Tracks the exact text we have typed into the focused app during a session.
/// On every update, computes the minimal diff: how many trailing chars to delete
/// (via backspaces) and what new tail to append (via paste).
#[derive(Debug, Default, Clone)]
pub struct TypedState {
    typed: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Diff {
    pub backspace_count: usize,
    pub append: String,
}

impl TypedState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> &str { &self.typed }

    /// Compute the diff needed to bring the typed text to `new_full`.
    /// Returns the diff and updates internal state.
    pub fn update(&mut self, new_full: &str) -> Diff {
        let common = common_prefix_len_chars(&self.typed, new_full);
        let typed_chars: usize = self.typed.chars().count();
        let backspace_count = typed_chars.saturating_sub(common);
        let append: String = new_full.chars().skip(common).collect();
        self.typed = new_full.to_string();
        Diff { backspace_count, append }
    }

    pub fn reset(&mut self) {
        self.typed.clear();
    }
}

fn common_prefix_len_chars(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_to_text() {
        let mut s = TypedState::new();
        let d = s.update("hello");
        assert_eq!(d, Diff { backspace_count: 0, append: "hello".into() });
        assert_eq!(s.current(), "hello");
    }

    #[test]
    fn append_only() {
        let mut s = TypedState::new();
        s.update("hello");
        let d = s.update("hello world");
        assert_eq!(d, Diff { backspace_count: 0, append: " world".into() });
    }

    #[test]
    fn replace_tail() {
        let mut s = TypedState::new();
        s.update("hello wrold");
        let d = s.update("hello world");
        assert_eq!(d, Diff { backspace_count: 5, append: "world".into() });
    }

    #[test]
    fn full_replacement() {
        let mut s = TypedState::new();
        s.update("foo");
        let d = s.update("bar");
        assert_eq!(d, Diff { backspace_count: 3, append: "bar".into() });
    }

    #[test]
    fn truncation_only() {
        let mut s = TypedState::new();
        s.update("hello world");
        let d = s.update("hello");
        assert_eq!(d, Diff { backspace_count: 6, append: "".into() });
    }

    #[test]
    fn unchanged() {
        let mut s = TypedState::new();
        s.update("same");
        let d = s.update("same");
        assert_eq!(d, Diff { backspace_count: 0, append: "".into() });
    }

    #[test]
    fn multibyte_chars_counted_correctly() {
        let mut s = TypedState::new();
        s.update("café");
        let d = s.update("cafés");
        assert_eq!(d, Diff { backspace_count: 0, append: "s".into() });

        let d2 = s.update("café");
        assert_eq!(d2, Diff { backspace_count: 1, append: "".into() });
    }

    #[test]
    fn reset_clears_state() {
        let mut s = TypedState::new();
        s.update("hello");
        s.reset();
        let d = s.update("world");
        assert_eq!(d, Diff { backspace_count: 0, append: "world".into() });
    }
}
```

- [ ] **Step 2: Write minimal `src/daemon/mod.rs`**

```rust
pub mod state;
```

- [ ] **Step 3: Wire it into `src/lib.rs`**

```rust
pub mod config;
pub mod daemon;
pub mod terms;
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib daemon::state
```

Expected: 8 passing tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(state): typed-state diff tracker with multibyte support"
```

---

### Task 5: VAD trait + chunking state machine

**Files:**
- Create: `src/daemon/vad.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write `src/daemon/vad.rs` with the trait, chunker, and mock-based tests**

```rust
//! Voice-activity-driven chunking.
//!
//! The `Vad` trait answers "is this 30ms frame speech?" — implemented by silero
//! (default, neural) and webrtc (fallback, classical). The `Chunker` state machine
//! consumes a stream of (frame, is_speech) and emits chunk-boundary events.

use crate::config::VadConfig;

/// Per-frame speech classifier. Frames are 16kHz mono i16 samples; the implementation
/// decides its own frame size (silero=512, webrtc=160/320/480 at 16kHz).
pub trait Vad: Send {
    /// Required input frame length, in samples.
    fn frame_samples(&self) -> usize;
    /// Classify one frame.
    fn is_speech(&mut self, frame: &[i16]) -> bool;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChunkEvent {
    /// A new chunk just started (speech began after silence).
    Start,
    /// Audio samples belonging to the current chunk.
    Samples(Vec<i16>),
    /// The current chunk just ended. Reason: long-enough silence, max length, or session close.
    End { reason: EndReason },
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum EndReason {
    SilenceTimeout,
    MaxLength,
    SessionClosed,
}

pub struct Chunker {
    sample_rate: u32,
    min_silence_samples: u64,
    max_chunk_samples: u64,
    in_speech: bool,
    silence_streak: u64,
    chunk_len: u64,
    buf: Vec<i16>,
}

impl Chunker {
    pub fn new(sample_rate: u32, cfg: &VadConfig) -> Self {
        Self {
            sample_rate,
            min_silence_samples: ms_to_samples(cfg.min_silence_ms, sample_rate),
            max_chunk_samples: ms_to_samples(cfg.max_chunk_ms, sample_rate),
            in_speech: false,
            silence_streak: 0,
            chunk_len: 0,
            buf: Vec::with_capacity(sample_rate as usize),
        }
    }

    /// Feed one frame. Returns 0..N events to emit, in order.
    pub fn feed(&mut self, frame: &[i16], is_speech: bool) -> Vec<ChunkEvent> {
        let mut events = Vec::new();
        let frame_len = frame.len() as u64;

        if !self.in_speech {
            if is_speech {
                self.in_speech = true;
                self.chunk_len = 0;
                self.silence_streak = 0;
                self.buf.clear();
                self.buf.extend_from_slice(frame);
                self.chunk_len += frame_len;
                events.push(ChunkEvent::Start);
            }
            // pure silence outside a chunk: drop the frame.
            return events;
        }

        // Inside a chunk
        self.buf.extend_from_slice(frame);
        self.chunk_len += frame_len;
        if is_speech {
            self.silence_streak = 0;
        } else {
            self.silence_streak += frame_len;
        }

        if self.silence_streak >= self.min_silence_samples {
            events.push(ChunkEvent::Samples(std::mem::take(&mut self.buf)));
            events.push(ChunkEvent::End { reason: EndReason::SilenceTimeout });
            self.in_speech = false;
            self.silence_streak = 0;
            self.chunk_len = 0;
        } else if self.chunk_len >= self.max_chunk_samples {
            events.push(ChunkEvent::Samples(std::mem::take(&mut self.buf)));
            events.push(ChunkEvent::End { reason: EndReason::MaxLength });
            // Immediately start a new chunk so audio isn't dropped
            self.in_speech = true;
            self.chunk_len = 0;
            self.silence_streak = 0;
            events.push(ChunkEvent::Start);
        }

        events
    }

    /// Close out any in-flight chunk (called on session end).
    pub fn close(&mut self) -> Vec<ChunkEvent> {
        if !self.in_speech { return vec![]; }
        let samples = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        if !samples.is_empty() {
            out.push(ChunkEvent::Samples(samples));
        }
        out.push(ChunkEvent::End { reason: EndReason::SessionClosed });
        self.in_speech = false;
        self.chunk_len = 0;
        self.silence_streak = 0;
        out
    }
}

fn ms_to_samples(ms: u32, sample_rate: u32) -> u64 {
    (ms as u64 * sample_rate as u64) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VadConfig;

    fn cfg(min_silence_ms: u32, max_chunk_ms: u32) -> VadConfig {
        VadConfig {
            backend: crate::config::VadBackend::Silero,
            min_silence_ms,
            max_chunk_ms,
        }
    }

    fn frame(n: usize) -> Vec<i16> { vec![0i16; n] }

    #[test]
    fn silence_then_speech_starts_chunk() {
        let mut c = Chunker::new(16000, &cfg(400, 5000));
        assert!(c.feed(&frame(160), false).is_empty());
        let evs = c.feed(&frame(160), true);
        assert_eq!(evs, vec![ChunkEvent::Start]);
    }

    #[test]
    fn silence_after_speech_ends_chunk() {
        let mut c = Chunker::new(16000, &cfg(100, 5000));  // 1600 samples silence
        c.feed(&frame(160), true);
        // 1600 samples of silence = 100ms
        for _ in 0..10 {
            let _ = c.feed(&frame(160), false);
        }
        // The 10th silence frame should have triggered End
        // Verify by checking we're no longer in speech
        let evs = c.feed(&frame(160), true);
        assert_eq!(evs[0], ChunkEvent::Start, "should be starting a new chunk after End");
    }

    #[test]
    fn max_chunk_length_forces_end_and_restart() {
        let mut c = Chunker::new(16000, &cfg(400, 100));  // 1600 sample max
        c.feed(&frame(160), true);  // Start
        let mut last = vec![];
        for _ in 0..20 {
            last = c.feed(&frame(160), true);
            if !last.is_empty() { break; }
        }
        // Should have Samples, End{MaxLength}, Start
        assert!(matches!(last[0], ChunkEvent::Samples(_)));
        assert_eq!(last[1], ChunkEvent::End { reason: EndReason::MaxLength });
        assert_eq!(last[2], ChunkEvent::Start);
    }

    #[test]
    fn close_during_silence_emits_nothing() {
        let mut c = Chunker::new(16000, &cfg(400, 5000));
        c.feed(&frame(160), false);
        assert!(c.close().is_empty());
    }

    #[test]
    fn close_during_speech_flushes_chunk() {
        let mut c = Chunker::new(16000, &cfg(400, 5000));
        c.feed(&frame(160), true);
        c.feed(&frame(160), true);
        let evs = c.close();
        assert!(matches!(evs[0], ChunkEvent::Samples(_)));
        assert_eq!(evs[1], ChunkEvent::End { reason: EndReason::SessionClosed });
    }
}
```

- [ ] **Step 2: Add the silero and webrtc impls (skeletons that defer to their crates)**

Append to `src/daemon/vad.rs`:

```rust
// ---- silero backend ---------------------------------------------------------

pub struct SileroVad {
    detector: voice_activity_detector::VoiceActivityDetector,
}

impl SileroVad {
    pub fn new(sample_rate: u32) -> anyhow::Result<Self> {
        let detector = voice_activity_detector::VoiceActivityDetector::builder()
            .sample_rate(sample_rate as i64)
            .chunk_size(512_usize)
            .build()?;
        Ok(Self { detector })
    }
}

impl Vad for SileroVad {
    fn frame_samples(&self) -> usize { 512 }
    fn is_speech(&mut self, frame: &[i16]) -> bool {
        self.detector.predict(frame.iter().copied()) > 0.5
    }
}

// ---- webrtc backend ---------------------------------------------------------

pub struct WebrtcVad {
    vad: webrtc_vad::Vad,
}

impl WebrtcVad {
    pub fn new(_sample_rate: u32) -> Self {
        let mut v = webrtc_vad::Vad::new();
        v.set_mode(webrtc_vad::VadMode::Aggressive);
        v.set_sample_rate(webrtc_vad::SampleRate::Rate16kHz);
        Self { vad: v }
    }
}

impl Vad for WebrtcVad {
    fn frame_samples(&self) -> usize { 480 }  // 30ms at 16kHz
    fn is_speech(&mut self, frame: &[i16]) -> bool {
        self.vad.is_voice_segment(frame).unwrap_or(false)
    }
}

pub fn make(cfg: &VadConfig, sample_rate: u32) -> anyhow::Result<Box<dyn Vad>> {
    use crate::config::VadBackend;
    Ok(match cfg.backend {
        VadBackend::Silero => Box::new(SileroVad::new(sample_rate)?),
        VadBackend::Webrtc => Box::new(WebrtcVad::new(sample_rate)),
    })
}
```

- [ ] **Step 3: Add `pub mod vad;` to `src/daemon/mod.rs`**

```rust
pub mod state;
pub mod vad;
```

- [ ] **Step 4: Run chunker tests**

```bash
cargo test --lib daemon::vad
```

Expected: 5 passing tests. The silero/webrtc backends are exercised in the manual end-to-end test, not unit tests (they need real audio).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(vad): trait + chunker state machine + silero/webrtc backends"
```

---

### Task 6: ASR client (OpenAI-compatible transcriptions)

**Files:**
- Create: `src/daemon/asr_client.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write `src/daemon/asr_client.rs` with the trait and impl**

```rust
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
```

- [ ] **Step 2: Wire it up in `src/daemon/mod.rs`**

```rust
pub mod asr_client;
pub mod state;
pub mod vad;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib daemon::asr_client
```

Expected: 3 passing tests.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(asr): OpenAI-compatible transcription client with wiremock tests"
```

---

### Task 7: Editor client (light + heavy passes)

**Files:**
- Create: `src/daemon/editor_client.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write `src/daemon/editor_client.rs`**

```rust
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
```

- [ ] **Step 2: Wire it up**

```rust
// src/daemon/mod.rs
pub mod asr_client;
pub mod editor_client;
pub mod state;
pub mod vad;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib daemon::editor_client
```

Expected: 3 passing tests.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(editor): light + heavy chat-completions client with term DB injection"
```

---

### Task 8: Platform trait + MockPlatform

**Files:**
- Create: `src/platform/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write `src/platform/mod.rs`**

```rust
//! Cross-platform abstraction for hotkey capture and synthetic input.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HotkeyEvent {
    Press,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSnapshot(pub Option<String>);

/// Platform-facing capabilities used by the daemon. All methods may run on
/// blocking threads internally; the trait surface is async so the pipeline can
/// `.await` them.
#[async_trait]
pub trait Platform: Send + Sync {
    /// Begin listening for the configured hotkey. Returns a receiver that yields
    /// Press/Release events. The internal thread runs for the lifetime of the
    /// returned receiver; dropping the receiver stops the thread.
    fn hotkey_stream(self: Arc<Self>, binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>>;

    async fn read_clipboard(&self) -> Result<ClipboardSnapshot>;
    async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()>;

    /// Send the paste keystroke (e.g. "Ctrl+V").
    async fn paste(&self, shortcut: &str) -> Result<()>;

    /// Send N backspaces.
    async fn backspace(&self, n: usize) -> Result<()>;
}

/// In-memory platform for tests. Always exported (cheap) so integration tests in
/// `tests/` can use it without a feature flag.
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, PartialEq, Eq, Clone)]
    pub enum Call {
        ReadClipboard,
        WriteClipboard(ClipboardSnapshot),
        Paste(String),
        Backspace(usize),
    }

    #[derive(Default)]
    pub struct MockPlatform {
        pub clipboard: Mutex<ClipboardSnapshot>,
        pub calls: Mutex<Vec<Call>>,
    }

    impl MockPlatform {
        pub fn new() -> Arc<Self> { Arc::new(Self::default()) }
        pub fn calls(&self) -> Vec<Call> { self.calls.lock().unwrap().clone() }
    }

    #[async_trait]
    impl Platform for MockPlatform {
        fn hotkey_stream(self: Arc<Self>, _binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
            let (_tx, rx) = mpsc::channel(8);
            Ok(rx)
        }
        async fn read_clipboard(&self) -> Result<ClipboardSnapshot> {
            self.calls.lock().unwrap().push(Call::ReadClipboard);
            Ok(self.clipboard.lock().unwrap().clone())
        }
        async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()> {
            self.calls.lock().unwrap().push(Call::WriteClipboard(snap.clone()));
            *self.clipboard.lock().unwrap() = snap.clone();
            Ok(())
        }
        async fn paste(&self, shortcut: &str) -> Result<()> {
            self.calls.lock().unwrap().push(Call::Paste(shortcut.into()));
            Ok(())
        }
        async fn backspace(&self, n: usize) -> Result<()> {
            self.calls.lock().unwrap().push(Call::Backspace(n));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;

    #[tokio::test]
    async fn mock_records_calls_in_order() {
        let p = MockPlatform::new();
        p.write_clipboard(&ClipboardSnapshot(Some("hi".into()))).await.unwrap();
        p.paste("Ctrl+V").await.unwrap();
        p.backspace(3).await.unwrap();
        assert_eq!(
            p.calls(),
            vec![
                Call::WriteClipboard(ClipboardSnapshot(Some("hi".into()))),
                Call::Paste("Ctrl+V".into()),
                Call::Backspace(3),
            ]
        );
    }
}
```

- [ ] **Step 2: Wire into `src/lib.rs`**

```rust
pub mod config;
pub mod daemon;
pub mod platform;
pub mod terms;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib platform::
```

Expected: 1 passing test.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(platform): trait + MockPlatform for dependency injection"
```

---

### Task 9: Linux platform — clipboard

**Files:**
- Create: `src/platform/linux/mod.rs`
- Create: `src/platform/linux/clipboard.rs`

- [ ] **Step 1: Write `src/platform/linux/clipboard.rs`**

```rust
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
```

- [ ] **Step 2: Write `src/platform/linux/mod.rs` (clipboard methods only for now)**

```rust
pub mod clipboard;
pub mod hotkey;   // Task 11
pub mod input;    // Task 10

use crate::platform::{ClipboardSnapshot, HotkeyEvent, Platform};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

pub struct LinuxPlatform;

impl LinuxPlatform {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Platform for LinuxPlatform {
    fn hotkey_stream(self: Arc<Self>, binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
        hotkey::listen(binding)
    }
    async fn read_clipboard(&self) -> Result<ClipboardSnapshot> {
        task::spawn_blocking(clipboard::read).await?
    }
    async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()> {
        let s = snap.clone();
        task::spawn_blocking(move || clipboard::write(&s)).await?
    }
    async fn paste(&self, shortcut: &str) -> Result<()> {
        let s = shortcut.to_string();
        task::spawn_blocking(move || input::paste(&s)).await?
    }
    async fn backspace(&self, n: usize) -> Result<()> {
        task::spawn_blocking(move || input::backspace(n)).await?
    }
}
```

- [ ] **Step 3: Confirm it compiles (modules `hotkey` and `input` are stubbed in next tasks)**

Stub `src/platform/linux/hotkey.rs`:

```rust
use crate::platform::HotkeyEvent;
use anyhow::Result;
use tokio::sync::mpsc;

pub fn listen(_binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
    anyhow::bail!("Linux hotkey listener not implemented yet (Task 11)")
}
```

Stub `src/platform/linux/input.rs`:

```rust
use anyhow::Result;

pub fn paste(_shortcut: &str) -> Result<()> {
    anyhow::bail!("Linux input synthesis not implemented yet (Task 10)")
}

pub fn backspace(_n: usize) -> Result<()> {
    anyhow::bail!("Linux input synthesis not implemented yet (Task 10)")
}
```

- [ ] **Step 4: Compile-check**

```bash
cargo build
```

Expected: compiles. (No new tests; clipboard is exercised manually + via Task 14 integration test.)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(linux): clipboard read/write via arboard + platform impl skeleton"
```

---

### Task 10: Linux platform — uinput paste/backspace

**Files:**
- Modify: `src/platform/linux/input.rs`
- Create: `99-localasr.rules` (repo root)

- [ ] **Step 1: Write the udev rule**

`99-localasr.rules` at repo root:

```
# Allow members of the `input` group to write to /dev/uinput (for localasr's
# synthetic-input feature). Apply with:
#   sudo cp 99-localasr.rules /etc/udev/rules.d/
#   sudo udevadm control --reload-rules
KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"
```

- [ ] **Step 2: Implement `src/platform/linux/input.rs`**

```rust
use anyhow::{Context, Result};
use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AttributeSet, EventType, InputEvent, Key,
};
use std::sync::Mutex;
use std::time::Duration;

static DEVICE: Mutex<Option<VirtualDevice>> = Mutex::new(None);

fn device() -> Result<std::sync::MutexGuard<'static, Option<VirtualDevice>>> {
    let mut g = DEVICE.lock().unwrap();
    if g.is_none() {
        let mut keys = AttributeSet::<Key>::new();
        for k in ALL_USED_KEYS {
            keys.insert(*k);
        }
        let dev = VirtualDeviceBuilder::new()
            .context("creating uinput device; ensure /dev/uinput is writable (see 99-localasr.rules)")?
            .name("localasr-virtual-kbd")
            .with_keys(&keys)?
            .build()?;
        *g = Some(dev);
        // Give udev a moment to set up the device node
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(g)
}

const ALL_USED_KEYS: &[Key] = &[
    Key::KEY_LEFTCTRL, Key::KEY_RIGHTCTRL,
    Key::KEY_LEFTSHIFT, Key::KEY_RIGHTSHIFT,
    Key::KEY_LEFTALT, Key::KEY_RIGHTALT,
    Key::KEY_LEFTMETA, Key::KEY_RIGHTMETA,
    Key::KEY_V, Key::KEY_BACKSPACE,
];

fn parse_chord(s: &str) -> Result<Vec<Key>> {
    let mut keys = Vec::new();
    for part in s.split('+').map(str::trim) {
        let k = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "leftctrl" => Key::KEY_LEFTCTRL,
            "rightctrl" => Key::KEY_RIGHTCTRL,
            "shift" | "leftshift" => Key::KEY_LEFTSHIFT,
            "rightshift" => Key::KEY_RIGHTSHIFT,
            "alt" | "leftalt" => Key::KEY_LEFTALT,
            "rightalt" => Key::KEY_RIGHTALT,
            "meta" | "super" | "leftmeta" => Key::KEY_LEFTMETA,
            "rightmeta" => Key::KEY_RIGHTMETA,
            "v" => Key::KEY_V,
            "backspace" | "back" => Key::KEY_BACKSPACE,
            other => anyhow::bail!("unsupported key in chord: {other}"),
        };
        keys.push(k);
    }
    if keys.is_empty() {
        anyhow::bail!("empty chord");
    }
    Ok(keys)
}

fn tap_chord(keys: &[Key]) -> Result<()> {
    let mut g = device()?;
    let dev = g.as_mut().unwrap();
    let down: Vec<InputEvent> = keys.iter()
        .map(|k| InputEvent::new(EventType::KEY, k.code(), 1))
        .collect();
    let up: Vec<InputEvent> = keys.iter().rev()
        .map(|k| InputEvent::new(EventType::KEY, k.code(), 0))
        .collect();
    dev.emit(&down)?;
    std::thread::sleep(Duration::from_millis(5));
    dev.emit(&up)?;
    Ok(())
}

pub fn paste(shortcut: &str) -> Result<()> {
    let keys = parse_chord(shortcut)?;
    tap_chord(&keys)
}

pub fn backspace(n: usize) -> Result<()> {
    let keys = vec![Key::KEY_BACKSPACE];
    for _ in 0..n {
        tap_chord(&keys)?;
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_key() {
        let k = parse_chord("V").unwrap();
        assert_eq!(k, vec![Key::KEY_V]);
    }

    #[test]
    fn parses_chord() {
        let k = parse_chord("Ctrl+V").unwrap();
        assert_eq!(k, vec![Key::KEY_LEFTCTRL, Key::KEY_V]);
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_chord("XYZ").is_err());
    }
}
```

- [ ] **Step 3: Run tests (parsing only — actual injection is manual)**

```bash
cargo test --lib platform::linux::input
```

Expected: 3 passing tests.

- [ ] **Step 4: Manual verification (skip if not on Linux)**

```bash
# Ensure udev rule is installed and you're in the `input` group (re-login if needed).
# Then run a quick injection test:
cargo run --release --example uinput_smoke   # add example if you want; otherwise:
cargo test --lib platform::linux::input -- --ignored uinput_live
```

(No automated test here; the manual checklist in Task 16 includes "press paste in a focus app and verify".)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(linux): uinput-based paste + backspace + chord parser"
```

---

### Task 11: Linux platform — evdev hotkey listener

**Files:**
- Modify: `src/platform/linux/hotkey.rs`

- [ ] **Step 1: Replace stub with real impl**

```rust
use crate::platform::HotkeyEvent;
use anyhow::{Context, Result};
use evdev::{Device, EventType, InputEventKind, Key};
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;
use tokio::sync::mpsc;

/// Parse a binding string like "RightCtrl" or "Ctrl+Alt+Space" into a set of
/// evdev Keys. The session is open while ALL keys in the set are held, and
/// closes the moment ANY key in the set is released.
fn parse_binding(s: &str) -> Result<HashSet<Key>> {
    let mut keys = HashSet::new();
    for part in s.split('+').map(str::trim) {
        let k = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "leftctrl" => Key::KEY_LEFTCTRL,
            "rightctrl" => Key::KEY_RIGHTCTRL,
            "shift" | "leftshift" => Key::KEY_LEFTSHIFT,
            "rightshift" => Key::KEY_RIGHTSHIFT,
            "alt" | "leftalt" => Key::KEY_LEFTALT,
            "rightalt" => Key::KEY_RIGHTALT,
            "meta" | "super" | "leftmeta" => Key::KEY_LEFTMETA,
            "rightmeta" => Key::KEY_RIGHTMETA,
            "space" => Key::KEY_SPACE,
            "f1" => Key::KEY_F1, "f2" => Key::KEY_F2, "f3" => Key::KEY_F3, "f4" => Key::KEY_F4,
            "f5" => Key::KEY_F5, "f6" => Key::KEY_F6, "f7" => Key::KEY_F7, "f8" => Key::KEY_F8,
            "f9" => Key::KEY_F9, "f10" => Key::KEY_F10, "f11" => Key::KEY_F11, "f12" => Key::KEY_F12,
            other => anyhow::bail!("unsupported hotkey key: {other}"),
        };
        keys.insert(k);
    }
    if keys.is_empty() {
        anyhow::bail!("empty binding");
    }
    Ok(keys)
}

fn enumerate_keyboards() -> Vec<(PathBuf, Device)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        for e in entries.flatten() {
            let p = e.path();
            if !p.to_string_lossy().contains("event") { continue; }
            if let Ok(dev) = Device::open(&p) {
                if dev.supported_keys().map(|k| k.contains(Key::KEY_A)).unwrap_or(false) {
                    out.push((p, dev));
                }
            }
        }
    }
    out
}

pub fn listen(binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
    let wanted = parse_binding(binding)?;
    let (tx, rx) = mpsc::channel(16);

    thread::spawn(move || {
        // Re-acquire devices every 2s on failure (per spec: "Hotkey device disappears")
        loop {
            let kbds = enumerate_keyboards();
            if kbds.is_empty() {
                tracing::warn!("no keyboards visible via /dev/input/event*; retrying in 2s");
                thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
            if let Err(e) = run_loop(&wanted, kbds, &tx) {
                tracing::warn!("hotkey loop ended: {e}; reacquiring in 2s");
                thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    });
    Ok(rx)
}

fn run_loop(wanted: &HashSet<Key>, mut kbds: Vec<(PathBuf, Device)>, tx: &mpsc::Sender<HotkeyEvent>) -> Result<()> {
    let mut held: HashSet<Key> = HashSet::new();
    let mut session_open = false;
    loop {
        for (_, dev) in kbds.iter_mut() {
            for ev in dev.fetch_events()? {
                if ev.event_type() != EventType::KEY { continue; }
                if let InputEventKind::Key(k) = ev.kind() {
                    if !wanted.contains(&k) { continue; }
                    match ev.value() {
                        1 => { held.insert(k); }            // press
                        2 => {}                              // autorepeat
                        0 => { held.remove(&k); }            // release
                        _ => {}
                    }
                    let all_held = wanted.is_subset(&held);
                    if all_held && !session_open {
                        session_open = true;
                        let _ = tx.blocking_send(HotkeyEvent::Press);
                    } else if !all_held && session_open {
                        session_open = false;
                        let _ = tx.blocking_send(HotkeyEvent::Release);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_key() {
        let k = parse_binding("RightCtrl").unwrap();
        assert!(k.contains(&Key::KEY_RIGHTCTRL));
        assert_eq!(k.len(), 1);
    }

    #[test]
    fn parses_chord() {
        let k = parse_binding("Ctrl+Alt+Space").unwrap();
        assert_eq!(k.len(), 3);
        assert!(k.contains(&Key::KEY_LEFTCTRL));
        assert!(k.contains(&Key::KEY_LEFTALT));
        assert!(k.contains(&Key::KEY_SPACE));
    }

    #[test]
    fn parses_function_key() {
        let k = parse_binding("F8").unwrap();
        assert!(k.contains(&Key::KEY_F8));
    }

    #[test]
    fn rejects_unknown() {
        assert!(parse_binding("FlibberHotkey").is_err());
    }
}
```

- [ ] **Step 2: Compile and test parser**

```bash
cargo test --lib platform::linux::hotkey
```

Expected: 4 passing tests. (Live `fetch_events` is exercised manually.)

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(linux): evdev hotkey listener with chord support and auto-reacquire"
```

---

### Task 12: Audio capture thread

**Files:**
- Create: `src/daemon/audio.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write `src/daemon/audio.rs`**

```rust
//! Audio capture: spins up a cpal input stream on a dedicated thread, sends
//! 16kHz mono i16 frames over a crossbeam channel.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread;

pub struct AudioCapture {
    _thread: thread::JoinHandle<()>,
    pub frames: Receiver<Vec<i16>>,
    stop_tx: Sender<()>,
}

impl AudioCapture {
    /// Start capturing from the named device (empty string = default).
    /// Resamples to `target_sample_rate` mono i16. Frames are arbitrary length;
    /// the consumer slices them into VAD-sized pieces.
    pub fn start(device_name: &str, target_sample_rate: u32) -> Result<Self> {
        let (frames_tx, frames_rx) = bounded::<Vec<i16>>(64);
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let device_name = device_name.to_string();

        let handle = thread::spawn(move || {
            if let Err(e) = run(device_name, target_sample_rate, frames_tx, stop_rx) {
                tracing::error!("audio capture thread crashed: {e:?}");
            }
        });

        Ok(Self { _thread: handle, frames: frames_rx, stop_tx })
    }

    pub fn stop(self) {
        let _ = self.stop_tx.send(());
    }
}

fn run(device_name: String, target_sr: u32, frames_tx: Sender<Vec<i16>>, stop_rx: Receiver<()>) -> Result<()> {
    let host = cpal::default_host();
    let device = if device_name.is_empty() {
        host.default_input_device().context("no default input device")?
    } else {
        host.input_devices()?
            .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
            .with_context(|| format!("input device not found: {device_name}"))?
    };
    let config = device.default_input_config().context("default input config")?;
    let source_sr = config.sample_rate().0;
    let channels = config.channels() as usize;

    let frames_tx_cb = frames_tx.clone();
    let stream = match config.sample_format() {
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data: &[i16], _| {
                let mono = downmix_i16(data, channels);
                let resampled = resample_naive(&mono, source_sr, target_sr);
                let _ = frames_tx_cb.send(resampled);
            },
            |e| tracing::error!("audio stream error: {e}"),
            None,
        )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| {
                let mono = downmix_f32(data, channels);
                let resampled = resample_naive_f32(&mono, source_sr, target_sr);
                let _ = frames_tx_cb.send(resampled);
            },
            |e| tracing::error!("audio stream error: {e}"),
            None,
        )?,
        _ => anyhow::bail!("unsupported audio sample format"),
    };

    stream.play()?;
    let _ = stop_rx.recv();
    drop(stream);
    Ok(())
}

fn downmix_i16(data: &[i16], channels: usize) -> Vec<i16> {
    if channels == 1 { return data.to_vec(); }
    data.chunks_exact(channels)
        .map(|c| (c.iter().map(|&x| x as i32).sum::<i32>() / channels as i32) as i16)
        .collect()
}

fn downmix_f32(data: &[f32], channels: usize) -> Vec<i16> {
    let scale = 32767.0;
    if channels == 1 {
        return data.iter().map(|&x| (x.clamp(-1.0, 1.0) * scale) as i16).collect();
    }
    data.chunks_exact(channels)
        .map(|c| {
            let avg: f32 = c.iter().sum::<f32>() / channels as f32;
            (avg.clamp(-1.0, 1.0) * scale) as i16
        })
        .collect()
}

/// Cheap nearest-neighbor resampling. Adequate for VAD + Whisper input which
/// tolerate poor resampling. Replace with `rubato` if quality matters later.
fn resample_naive(src: &[i16], src_sr: u32, dst_sr: u32) -> Vec<i16> {
    if src_sr == dst_sr { return src.to_vec(); }
    let ratio = src_sr as f64 / dst_sr as f64;
    let out_len = ((src.len() as f64) / ratio).round() as usize;
    (0..out_len).map(|i| {
        let idx = ((i as f64) * ratio) as usize;
        src[idx.min(src.len() - 1)]
    }).collect()
}

fn resample_naive_f32(src: &[i16], src_sr: u32, dst_sr: u32) -> Vec<i16> {
    resample_naive(src, src_sr, dst_sr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_stereo_to_mono() {
        let stereo = vec![100, -100, 200, -200, 50, 50];
        let mono = downmix_i16(&stereo, 2);
        assert_eq!(mono, vec![0, 0, 50]);
    }

    #[test]
    fn resample_passthrough_when_equal() {
        let s = vec![1i16, 2, 3, 4];
        assert_eq!(resample_naive(&s, 16000, 16000), s);
    }

    #[test]
    fn resample_halves_when_double_rate() {
        let s = vec![1i16, 2, 3, 4, 5, 6, 7, 8];
        let r = resample_naive(&s, 32000, 16000);
        assert_eq!(r.len(), 4);
    }
}
```

- [ ] **Step 2: Wire in `src/daemon/mod.rs`**

```rust
pub mod asr_client;
pub mod audio;
pub mod editor_client;
pub mod state;
pub mod vad;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib daemon::audio
```

Expected: 3 passing tests. (Live capture is exercised in the Task 16 manual checklist.)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(audio): cpal capture thread with naive resampling + downmix"
```

---

### Task 13: Windows platform stubs (compiles, manual runtime test)

**Files:**
- Create: `src/platform/windows/mod.rs`
- Create: `src/platform/windows/hotkey.rs`
- Create: `src/platform/windows/input.rs`
- Create: `src/platform/windows/clipboard.rs`

- [ ] **Step 1: Write `src/platform/windows/clipboard.rs`**

```rust
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
```

- [ ] **Step 2: Write `src/platform/windows/input.rs`**

```rust
use anyhow::Result;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
    VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12,
};

fn parse_key(s: &str) -> Result<VIRTUAL_KEY> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "leftctrl" => VK_LCONTROL,
        "rightctrl" => VK_RCONTROL,
        "shift" => VK_SHIFT,
        "leftshift" => VK_LSHIFT,
        "rightshift" => VK_RSHIFT,
        "alt" => VK_MENU,
        "leftalt" => VK_LMENU,
        "rightalt" => VK_RMENU,
        "meta" | "super" | "leftmeta" => VK_LWIN,
        "rightmeta" => VK_RWIN,
        "space" => VK_SPACE,
        "backspace" | "back" => VK_BACK,
        "f1" => VK_F1, "f2" => VK_F2, "f3" => VK_F3, "f4" => VK_F4,
        "f5" => VK_F5, "f6" => VK_F6, "f7" => VK_F7, "f8" => VK_F8,
        "f9" => VK_F9, "f10" => VK_F10, "f11" => VK_F11, "f12" => VK_F12,
        c if c.len() == 1 => {
            let ch = c.chars().next().unwrap().to_ascii_uppercase();
            VIRTUAL_KEY(ch as u16)
        }
        other => anyhow::bail!("unsupported key: {other}"),
    })
}

fn key_event(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn tap_chord(parts: &[VIRTUAL_KEY]) -> Result<()> {
    let mut events: Vec<INPUT> = parts.iter().map(|k| key_event(*k, false)).collect();
    for k in parts.iter().rev() { events.push(key_event(*k, true)); }
    unsafe {
        let sent = SendInput(&events, std::mem::size_of::<INPUT>() as i32);
        if sent as usize != events.len() {
            anyhow::bail!("SendInput sent {} of {} events", sent, events.len());
        }
    }
    std::thread::sleep(Duration::from_millis(2));
    Ok(())
}

pub fn paste(shortcut: &str) -> Result<()> {
    let keys: Result<Vec<_>> = shortcut.split('+').map(str::trim).map(parse_key).collect();
    tap_chord(&keys?)
}

pub fn backspace(n: usize) -> Result<()> {
    for _ in 0..n {
        tap_chord(&[VK_BACK])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ctrl_v() {
        let k = parse_key("Ctrl").unwrap();
        assert_eq!(k, VK_CONTROL);
        let k = parse_key("V").unwrap();
        assert_eq!(k.0, b'V' as u16);
    }
}
```

- [ ] **Step 3: Write `src/platform/windows/hotkey.rs`**

```rust
use crate::platform::HotkeyEvent;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Mutex;
use std::thread;
use tokio::sync::mpsc;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
    VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    HHOOK, HOOKPROC, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

struct State {
    wanted: HashSet<u16>,
    held: HashSet<u16>,
    open: bool,
    tx: mpsc::Sender<HotkeyEvent>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn parse_binding(s: &str) -> Result<HashSet<u16>> {
    let mut keys = HashSet::new();
    for part in s.split('+').map(str::trim) {
        let vk = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => VK_CONTROL,
            "leftctrl" => VK_LCONTROL,
            "rightctrl" => VK_RCONTROL,
            "shift" => VK_SHIFT,
            "leftshift" => VK_LSHIFT,
            "rightshift" => VK_RSHIFT,
            "alt" => VK_MENU,
            "leftalt" => VK_LMENU,
            "rightalt" => VK_RMENU,
            "meta" | "super" | "leftmeta" => VK_LWIN,
            "rightmeta" => VK_RWIN,
            "space" => VK_SPACE,
            "f1" => VK_F1, "f2" => VK_F2, "f3" => VK_F3, "f4" => VK_F4,
            "f5" => VK_F5, "f6" => VK_F6, "f7" => VK_F7, "f8" => VK_F8,
            "f9" => VK_F9, "f10" => VK_F10, "f11" => VK_F11, "f12" => VK_F12,
            c if c.len() == 1 => VIRTUAL_KEY(c.chars().next().unwrap().to_ascii_uppercase() as u16),
            other => anyhow::bail!("unsupported hotkey: {other}"),
        };
        keys.insert(vk.0);
    }
    Ok(keys)
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kbd = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = kbd.vkCode as u16;
        let msg = wparam.0 as u32;
        let mut guard = STATE.lock().unwrap();
        if let Some(st) = guard.as_mut() {
            if st.wanted.contains(&vk) {
                match msg {
                    m if m == WM_KEYDOWN || m == WM_SYSKEYDOWN => { st.held.insert(vk); }
                    m if m == WM_KEYUP || m == WM_SYSKEYUP => { st.held.remove(&vk); }
                    _ => {}
                }
                let all = st.wanted.is_subset(&st.held);
                if all && !st.open {
                    st.open = true;
                    let _ = st.tx.try_send(HotkeyEvent::Press);
                } else if !all && st.open {
                    st.open = false;
                    let _ = st.tx.try_send(HotkeyEvent::Release);
                }
            }
        }
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

pub fn listen(binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
    let wanted = parse_binding(binding)?;
    let (tx, rx) = mpsc::channel(16);
    *STATE.lock().unwrap() = Some(State { wanted, held: HashSet::new(), open: false, tx });

    thread::spawn(|| unsafe {
        let hmod = GetModuleHandleW(None).unwrap_or_default();
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod.into(), 0)
            .expect("SetWindowsHookExW failed");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_chord() {
        let k = parse_binding("Ctrl+Alt+Space").unwrap();
        assert_eq!(k.len(), 3);
    }
}
```

- [ ] **Step 4: Write `src/platform/windows/mod.rs`**

```rust
pub mod clipboard;
pub mod hotkey;
pub mod input;

use crate::platform::{ClipboardSnapshot, HotkeyEvent, Platform};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

pub struct WindowsPlatform;

impl WindowsPlatform {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Platform for WindowsPlatform {
    fn hotkey_stream(self: Arc<Self>, binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>> {
        hotkey::listen(binding)
    }
    async fn read_clipboard(&self) -> Result<ClipboardSnapshot> {
        task::spawn_blocking(clipboard::read).await?
    }
    async fn write_clipboard(&self, snap: &ClipboardSnapshot) -> Result<()> {
        let s = snap.clone();
        task::spawn_blocking(move || clipboard::write(&s)).await?
    }
    async fn paste(&self, shortcut: &str) -> Result<()> {
        let s = shortcut.to_string();
        task::spawn_blocking(move || input::paste(&s)).await?
    }
    async fn backspace(&self, n: usize) -> Result<()> {
        task::spawn_blocking(move || input::backspace(n)).await?
    }
}
```

- [ ] **Step 5: Verify cross-compilation**

```bash
rustup target add x86_64-pc-windows-gnu
cargo check --target x86_64-pc-windows-gnu
```

Expected: compiles. (If linker errors only, that's OK — actual Windows runtime testing happens in Task 16's manual checklist when you have a Windows box.)

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(windows): platform impl via SendInput + WH_KEYBOARD_LL + arboard"
```

---

### Task 14: Injector

**Files:**
- Create: `src/daemon/injector.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write `src/daemon/injector.rs`**

```rust
//! Combines clipboard save/restore + paste + backspace into single high-level
//! operations. Exists as its own module so the orchestration is independently
//! testable against `MockPlatform`.

use crate::config::InjectionConfig;
use crate::daemon::state::Diff;
use crate::platform::{ClipboardSnapshot, Platform};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

pub struct Injector {
    cfg: InjectionConfig,
    platform: Arc<dyn Platform>,
}

impl Injector {
    pub fn new(cfg: InjectionConfig, platform: Arc<dyn Platform>) -> Self {
        Self { cfg, platform }
    }

    /// Apply a single diff: backspace N times, then paste the new tail (if any).
    /// `saved` is the clipboard snapshot we restore to afterwards.
    pub async fn apply(&self, diff: &Diff, saved: &ClipboardSnapshot) -> Result<()> {
        if diff.backspace_count == 0 && diff.append.is_empty() {
            return Ok(());
        }
        if diff.backspace_count > 0 {
            self.platform.backspace(diff.backspace_count).await?;
        }
        if !diff.append.is_empty() {
            self.platform.write_clipboard(&ClipboardSnapshot(Some(diff.append.clone()))).await?;
            self.platform.paste(&self.cfg.paste_shortcut).await?;
            sleep(Duration::from_millis(self.cfg.restore_delay_ms)).await;
            self.platform.write_clipboard(saved).await?;
        }
        Ok(())
    }

    pub async fn save_clipboard(&self) -> Result<ClipboardSnapshot> {
        self.platform.read_clipboard().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::{Call, MockPlatform};

    fn cfg() -> InjectionConfig {
        InjectionConfig {
            mode: "clipboard_paste".into(),
            paste_shortcut: "Ctrl+V".into(),
            restore_delay_ms: 1,
        }
    }

    #[tokio::test]
    async fn append_only_pastes_and_restores() {
        let p = MockPlatform::new();
        let saved = ClipboardSnapshot(Some("original".into()));
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 0, append: "hello".into() }, &saved).await.unwrap();
        let calls = p.calls();
        assert_eq!(calls[0], Call::WriteClipboard(ClipboardSnapshot(Some("hello".into()))));
        assert_eq!(calls[1], Call::Paste("Ctrl+V".into()));
        assert_eq!(calls[2], Call::WriteClipboard(saved));
    }

    #[tokio::test]
    async fn backspace_only_does_not_touch_clipboard() {
        let p = MockPlatform::new();
        let saved = ClipboardSnapshot(Some("x".into()));
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 3, append: "".into() }, &saved).await.unwrap();
        let calls = p.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], Call::Backspace(3));
    }

    #[tokio::test]
    async fn backspace_then_paste() {
        let p = MockPlatform::new();
        let saved = ClipboardSnapshot(None);
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 5, append: "world".into() }, &saved).await.unwrap();
        let calls = p.calls();
        assert_eq!(calls[0], Call::Backspace(5));
        assert_eq!(calls[1], Call::WriteClipboard(ClipboardSnapshot(Some("world".into()))));
        assert_eq!(calls[2], Call::Paste("Ctrl+V".into()));
        assert_eq!(calls[3], Call::WriteClipboard(saved));
    }

    #[tokio::test]
    async fn empty_diff_is_noop() {
        let p = MockPlatform::new();
        let inj = Injector::new(cfg(), p.clone());
        inj.apply(&Diff { backspace_count: 0, append: "".into() }, &ClipboardSnapshot(None)).await.unwrap();
        assert!(p.calls().is_empty());
    }
}
```

- [ ] **Step 2: Wire in `src/daemon/mod.rs`**

```rust
pub mod asr_client;
pub mod audio;
pub mod editor_client;
pub mod injector;
pub mod state;
pub mod vad;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib daemon::injector
```

Expected: 4 passing tests.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(injector): clipboard-save/paste/restore + backspace orchestration"
```

---

### Task 15: Pipeline orchestrator

**Files:**
- Create: `src/daemon/pipeline.rs`
- Modify: `src/daemon/mod.rs`

- [ ] **Step 1: Write `src/daemon/pipeline.rs`**

```rust
//! Per-session orchestration: consumes ChunkEvents from the chunker, calls ASR
//! + light editor per chunk, runs heavy editor on close. Applies every text
//! change via the Injector.

use crate::config::{Config, EditorConfig};
use crate::daemon::asr_client::Asr;
use crate::daemon::editor_client::Editor;
use crate::daemon::injector::Injector;
use crate::daemon::state::TypedState;
use crate::daemon::vad::{ChunkEvent, EndReason};
use crate::platform::ClipboardSnapshot;
use crate::terms::TermDb;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct Pipeline {
    pub cfg: Config,
    pub asr: Arc<dyn Asr>,
    pub editor: Arc<dyn Editor>,
    pub injector: Arc<Injector>,
    pub terms: Option<Arc<TermDb>>,
}

impl Pipeline {
    /// Run one full session. Consumes events until the channel closes.
    /// Returns when the channel closes (i.e., session aborted or audio ended).
    pub async fn run(&self, mut events: mpsc::Receiver<ChunkEvent>) -> Result<()> {
        let mut state = TypedState::new();
        let mut chunks_text: Vec<String> = Vec::new();
        let saved = self.injector.save_clipboard().await?;

        while let Some(ev) = events.recv().await {
            match ev {
                ChunkEvent::Start => {}
                ChunkEvent::Samples(samples) => {
                    let raw = match self.asr.transcribe(&samples).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("ASR failed: {e}; skipping chunk");
                            continue;
                        }
                    };
                    chunks_text.push(raw);
                    let polished = if self.cfg.editor.light.enabled {
                        let n = self.cfg.editor.light.context_chunks as usize;
                        let start = chunks_text.len().saturating_sub(n);
                        let window = &chunks_text[start..];
                        match self.editor.polish_light(window).await {
                            Ok(t) => {
                                // The light pass returns the corrected version of the LAST chunk.
                                *chunks_text.last_mut().unwrap() = t.clone();
                                t
                            }
                            Err(e) => {
                                tracing::warn!("light editor failed: {e}; using raw ASR");
                                chunks_text.last().cloned().unwrap_or_default()
                            }
                        }
                    } else {
                        chunks_text.last().cloned().unwrap_or_default()
                    };
                    // Compute the new full text and apply the diff.
                    let _ = polished;
                    let new_full = chunks_text.join(" ");
                    let diff = state.update(&new_full);
                    self.injector.apply(&diff, &saved).await?;
                }
                ChunkEvent::End { reason } => {
                    if reason == EndReason::SessionClosed {
                        self.finalize(&mut state, &chunks_text, &saved).await?;
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    async fn finalize(&self, state: &mut TypedState, chunks: &[String], saved: &ClipboardSnapshot) -> Result<()> {
        if !self.cfg.editor.heavy.enabled || chunks.is_empty() { return Ok(()); }
        let utterance = chunks.join(" ");
        let terms_ref = self.terms.as_deref();
        match self.editor.polish_heavy(&utterance, terms_ref).await {
            Ok(t) => {
                let diff = state.update(&t);
                self.injector.apply(&diff, saved).await?;
            }
            Err(e) => tracing::warn!("heavy editor failed: {e}"),
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn _enforce_editor_config_lifetime(c: &EditorConfig) -> &str { &c.model }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AsrConfig, AudioConfig, EditorPassConfig, HotkeyConfig, InjectionConfig, TermsConfig, VadBackend, VadConfig};
    use crate::daemon::vad::{ChunkEvent, EndReason};
    use crate::platform::mock::MockPlatform;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeAsr { transcripts: Mutex<std::vec::IntoIter<String>> }
    impl FakeAsr {
        fn new(xs: Vec<&str>) -> Self {
            Self { transcripts: Mutex::new(xs.into_iter().map(String::from).collect::<Vec<_>>().into_iter()) }
        }
    }
    #[async_trait]
    impl Asr for FakeAsr {
        async fn transcribe(&self, _: &[i16]) -> Result<String> {
            Ok(self.transcripts.lock().unwrap().next().unwrap_or_default())
        }
    }

    struct FakeEditor;
    #[async_trait]
    impl Editor for FakeEditor {
        async fn polish_light(&self, chunks: &[String]) -> Result<String> {
            Ok(chunks.last().cloned().unwrap_or_default())
        }
        async fn polish_heavy(&self, utt: &str, _: Option<&TermDb>) -> Result<String> {
            Ok(format!("{utt}."))
        }
    }

    fn test_cfg() -> Config {
        Config {
            hotkey: HotkeyConfig { binding: "RightCtrl".into() },
            audio: AudioConfig { device: "".into(), sample_rate: 16000 },
            vad: VadConfig { backend: VadBackend::Silero, min_silence_ms: 400, max_chunk_ms: 5000 },
            asr: AsrConfig { base_url: "".into(), api_key: "".into(), model: "".into(), language: "".into(), timeout_ms: 5000 },
            editor: EditorConfig {
                base_url: "".into(), api_key: "".into(), model: "".into(),
                light_temperature: 0.0, heavy_temperature: 0.2, light_timeout_ms: 3000, heavy_timeout_ms: 10000,
                light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
                heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
            },
            terms: TermsConfig { enabled: false, path: "".into() },
            injection: InjectionConfig { mode: "clipboard_paste".into(), paste_shortcut: "Ctrl+V".into(), restore_delay_ms: 1 },
        }
    }

    #[tokio::test]
    async fn full_session_runs_light_then_heavy() {
        let platform = MockPlatform::new();
        let inj = Arc::new(Injector::new(test_cfg().injection.clone(), platform.clone()));
        let pipeline = Pipeline {
            cfg: test_cfg(),
            asr: Arc::new(FakeAsr::new(vec!["hello", "world"])),
            editor: Arc::new(FakeEditor),
            injector: inj,
            terms: None,
        };
        let (tx, rx) = mpsc::channel(16);
        tx.send(ChunkEvent::Start).await.unwrap();
        tx.send(ChunkEvent::Samples(vec![0; 1600])).await.unwrap();
        tx.send(ChunkEvent::End { reason: EndReason::SilenceTimeout }).await.unwrap();
        tx.send(ChunkEvent::Start).await.unwrap();
        tx.send(ChunkEvent::Samples(vec![0; 1600])).await.unwrap();
        tx.send(ChunkEvent::End { reason: EndReason::SessionClosed }).await.unwrap();
        drop(tx);

        pipeline.run(rx).await.unwrap();

        // Final clipboard restore must equal the original (None — MockPlatform starts empty).
        // The last WriteClipboard call should restore the saved snapshot.
        let calls = platform.calls();
        let writes: Vec<_> = calls.iter().filter_map(|c| match c {
            crate::platform::mock::Call::WriteClipboard(s) => Some(s.clone()),
            _ => None,
        }).collect();
        // First write = "hello", then restore to None, then "world", then restore, then heavy "hello world.", then restore.
        assert!(writes.first().unwrap().0.as_deref().unwrap_or("").contains("hello"));
        assert_eq!(writes.last().unwrap(), &ClipboardSnapshot(None));
    }
}
```

- [ ] **Step 2: Wire in `src/daemon/mod.rs`**

```rust
pub mod asr_client;
pub mod audio;
pub mod editor_client;
pub mod injector;
pub mod pipeline;
pub mod state;
pub mod vad;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib daemon::pipeline
```

Expected: 1 passing test.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(pipeline): session orchestrator with light/heavy editor passes"
```

---

### Task 16: Daemon lifecycle + first-run wiring + manual checklist

**Files:**
- Create: `src/daemon/lifecycle.rs`
- Modify: `src/daemon/mod.rs`
- Modify: `src/main.rs`
- Modify: `README.md`

- [ ] **Step 1: Write `src/daemon/lifecycle.rs`**

```rust
//! The top-level daemon loop: subscribe to hotkey events, manage sessions,
//! handle abort-on-re-press.

use crate::config::Config;
use crate::daemon::asr_client::{Asr, OpenAiAsr};
use crate::daemon::audio::AudioCapture;
use crate::daemon::editor_client::{Editor, OpenAiEditor};
use crate::daemon::injector::Injector;
use crate::daemon::pipeline::Pipeline;
use crate::daemon::vad::{Chunker, Vad};
use crate::platform::{HotkeyEvent, Platform};
use crate::terms::TermDb;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub async fn run(cfg: Config, platform: Arc<dyn Platform>) -> Result<()> {
    let terms = if cfg.terms.enabled {
        let path = shellexpand::tilde(&cfg.terms.path).to_string();
        let db = crate::terms::load(std::path::Path::new(&path))
            .with_context(|| "term DB enabled but failed to load")?;
        Some(Arc::new(db))
    } else { None };

    let asr: Arc<dyn Asr> = Arc::new(OpenAiAsr::new(cfg.asr.clone())?);
    let editor: Arc<dyn Editor> = Arc::new(OpenAiEditor::new(cfg.editor.clone())?);
    let injector = Arc::new(Injector::new(cfg.injection.clone(), platform.clone()));

    let mut hotkey_rx = platform.clone().hotkey_stream(&cfg.hotkey.binding)?;
    tracing::info!("daemon ready; hotkey={}", cfg.hotkey.binding);

    let mut current: Option<(JoinHandle<()>, mpsc::Sender<()>)> = None;
    while let Some(ev) = hotkey_rx.recv().await {
        match ev {
            HotkeyEvent::Press => {
                if let Some((handle, abort)) = current.take() {
                    let _ = abort.send(()).await;
                    handle.abort();
                }
                let pipeline = Pipeline {
                    cfg: cfg.clone(),
                    asr: asr.clone(),
                    editor: editor.clone(),
                    injector: injector.clone(),
                    terms: terms.clone(),
                };
                let cfg2 = cfg.clone();
                let (abort_tx, mut abort_rx) = mpsc::channel::<()>(1);
                let handle = tokio::spawn(async move {
                    let res = tokio::select! {
                        r = run_session(pipeline, cfg2) => r,
                        _ = abort_rx.recv() => Ok(()),
                    };
                    if let Err(e) = res {
                        tracing::warn!("session ended with error: {e:?}");
                    }
                });
                current = Some((handle, abort_tx));
            }
            HotkeyEvent::Release => {
                // The session decides when to close (audio thread sees Release via a separate channel)
                // For simplicity in v1, we close the session immediately on release.
                if let Some((handle, abort)) = current.take() {
                    let _ = abort.send(()).await;
                    let _ = handle.await;
                }
            }
        }
    }
    Ok(())
}

async fn run_session(pipeline: Pipeline, cfg: Config) -> Result<()> {
    let capture = AudioCapture::start(&cfg.audio.device, cfg.audio.sample_rate)?;
    let (chunk_tx, chunk_rx) = mpsc::channel::<crate::daemon::vad::ChunkEvent>(64);

    let cfg_clone = cfg.clone();
    let frames = capture.frames.clone();
    let chunk_tx_clone = chunk_tx.clone();
    let vad_handle = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut vad = crate::daemon::vad::make(&cfg_clone.vad, cfg_clone.audio.sample_rate)?;
        let mut chunker = Chunker::new(cfg_clone.audio.sample_rate, &cfg_clone.vad);
        let mut leftover: Vec<i16> = Vec::new();
        let frame_size = vad.frame_samples();
        for incoming in frames.iter() {
            leftover.extend(incoming);
            while leftover.len() >= frame_size {
                let frame: Vec<i16> = leftover.drain(..frame_size).collect();
                let speech = vad.is_speech(&frame);
                for ev in chunker.feed(&frame, speech) {
                    let _ = chunk_tx_clone.blocking_send(ev);
                }
            }
        }
        for ev in chunker.close() {
            let _ = chunk_tx_clone.blocking_send(ev);
        }
        Ok(())
    });

    let result = pipeline.run(chunk_rx).await;
    capture.stop();
    let _ = vad_handle.await;
    result
}
```

- [ ] **Step 2: Wire in `src/daemon/mod.rs`**

```rust
pub mod asr_client;
pub mod audio;
pub mod editor_client;
pub mod injector;
pub mod lifecycle;
pub mod pipeline;
pub mod state;
pub mod vad;

pub use lifecycle::run;
```

- [ ] **Step 3: Update `src/main.rs` to load config and run daemon**

```rust
use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "localasr", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    Daemon,
    Setup,
    Doctor,
    Extract { folder: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Cmd::Daemon) {
        Cmd::Daemon => run_daemon().await,
        Cmd::Setup => anyhow::bail!("`localasr setup` lands in Plan 3"),
        Cmd::Doctor => anyhow::bail!("`localasr doctor` lands in Plan 2"),
        Cmd::Extract { .. } => anyhow::bail!("`localasr extract` lands in Plan 4"),
    }
}

async fn run_daemon() -> anyhow::Result<()> {
    let path = localasr::config::default_config_path()?;
    if !path.exists() {
        anyhow::bail!(
            "No config found at {}. Run: localasr setup",
            path.display()
        );
    }
    let cfg = localasr::config::load(&path)?;

    #[cfg(target_os = "linux")]
    let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::linux::LinuxPlatform::new());
    #[cfg(target_os = "windows")]
    let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::windows::WindowsPlatform::new());

    localasr::daemon::run(cfg, platform).await
}
```

- [ ] **Step 4: Add a manual verification section to `README.md`**

Append:

```markdown
## Manual verification checklist (Plan 1)

Before tagging a release, all four must pass:

### Linux / X11 (Xorg session)
- [ ] `localasr daemon` starts without error after a hand-written `config.toml`
- [ ] Holding RightCtrl + speaking inserts polished text into Firefox URL bar
- [ ] Holding RightCtrl + speaking inserts text into VS Code editor
- [ ] Holding RightCtrl + speaking with a target app *unfocused* causes no panic; original clipboard restored
- [ ] Releasing RightCtrl runs the heavy editor and updates the text
- [ ] Re-pressing RightCtrl mid-correction starts a new session

### Linux / Wayland (GNOME)
- [ ] Same checks as X11, against GNOME Terminal, Firefox, gedit

### Linux / Wayland (KDE)
- [ ] Same checks as X11, against Konsole, Firefox, Kate

### Windows 11
- [ ] `localasr.exe daemon` starts without error
- [ ] Holding RightCtrl + speaking inserts polished text into Notepad
- [ ] Same checks for Edge browser address bar and VS Code

Use `RUST_LOG=localasr=debug localasr daemon` for verbose logging.
```

- [ ] **Step 5: Verify the daemon binary compiles and starts (and exits cleanly with the no-config message)**

```bash
cargo build
# Remove any existing config to test the first-run path:
rm -f "$(cargo run --quiet -- --help >/dev/null; echo "")$HOME/.config/localasr/config.toml"
cargo run -- daemon
```

Expected: exits with message `No config found at <path>. Run: localasr setup`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(daemon): lifecycle wiring + first-run UX + manual checklist"
```

---

### Task 17: End-to-end integration test

**Files:**
- Create: `tests/end_to_end.rs`

- [ ] **Step 1: Add the integration test**

```rust
//! Drives the pipeline directly with mocks for ASR, editor, and platform.
//! Exercises the same code paths the daemon takes, minus real hotkey + audio.

use localasr::config::*;
use localasr::daemon::asr_client::Asr;
use localasr::daemon::editor_client::Editor;
use localasr::daemon::injector::Injector;
use localasr::daemon::pipeline::Pipeline;
use localasr::daemon::vad::{ChunkEvent, EndReason};
use localasr::platform::mock::{Call, MockPlatform};
use localasr::platform::ClipboardSnapshot;
use localasr::terms::TermDb;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct ScriptedAsr(Mutex<std::vec::IntoIter<String>>);
#[async_trait]
impl Asr for ScriptedAsr {
    async fn transcribe(&self, _: &[i16]) -> anyhow::Result<String> {
        Ok(self.0.lock().unwrap().next().unwrap_or_default())
    }
}

struct ScriptedEditor;
#[async_trait]
impl Editor for ScriptedEditor {
    async fn polish_light(&self, chunks: &[String]) -> anyhow::Result<String> {
        Ok(chunks.last().cloned().unwrap_or_default())
    }
    async fn polish_heavy(&self, utt: &str, _: Option<&TermDb>) -> anyhow::Result<String> {
        Ok(utt.trim().to_string() + ".")
    }
}

fn cfg() -> Config {
    Config {
        hotkey: HotkeyConfig { binding: "RightCtrl".into() },
        audio: AudioConfig { device: "".into(), sample_rate: 16000 },
        vad: VadConfig { backend: VadBackend::Silero, min_silence_ms: 400, max_chunk_ms: 5000 },
        asr: AsrConfig { base_url: "".into(), api_key: "".into(), model: "".into(), language: "".into(), timeout_ms: 5000 },
        editor: EditorConfig {
            base_url: "".into(), api_key: "".into(), model: "".into(),
            light_temperature: 0.0, heavy_temperature: 0.2, light_timeout_ms: 3000, heavy_timeout_ms: 10000,
            light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
            heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
        },
        terms: TermsConfig { enabled: false, path: "".into() },
        injection: InjectionConfig { mode: "clipboard_paste".into(), paste_shortcut: "Ctrl+V".into(), restore_delay_ms: 1 },
    }
}

#[tokio::test]
async fn two_chunks_then_heavy_pass_produces_correct_call_sequence() {
    let platform = MockPlatform::new();
    let inj = Arc::new(Injector::new(cfg().injection.clone(), platform.clone()));
    let pipeline = Pipeline {
        cfg: cfg(),
        asr: Arc::new(ScriptedAsr(Mutex::new(vec!["hello".to_string(), "world".to_string()].into_iter()))),
        editor: Arc::new(ScriptedEditor),
        injector: inj,
        terms: None,
    };

    let (tx, rx) = mpsc::channel(16);
    tx.send(ChunkEvent::Start).await.unwrap();
    tx.send(ChunkEvent::Samples(vec![0; 1600])).await.unwrap();
    tx.send(ChunkEvent::End { reason: EndReason::SilenceTimeout }).await.unwrap();
    tx.send(ChunkEvent::Start).await.unwrap();
    tx.send(ChunkEvent::Samples(vec![0; 1600])).await.unwrap();
    tx.send(ChunkEvent::End { reason: EndReason::SessionClosed }).await.unwrap();
    drop(tx);

    pipeline.run(rx).await.unwrap();

    let calls = platform.calls();
    // First call is always: read clipboard for save.
    assert_eq!(calls[0], Call::ReadClipboard);
    // Last clipboard-write must restore to the saved value (None for a fresh mock).
    let last_write = calls.iter().rev().find_map(|c| match c {
        Call::WriteClipboard(s) => Some(s.clone()),
        _ => None,
    }).unwrap();
    assert_eq!(last_write, ClipboardSnapshot(None));
    // Heavy pass should have produced a paste with trailing period.
    let heavy_paste = calls.iter().rev().find_map(|c| match c {
        Call::WriteClipboard(ClipboardSnapshot(Some(t))) if t.ends_with('.') => Some(t.clone()),
        _ => None,
    });
    assert!(heavy_paste.is_some(), "heavy editor's pasted text should end with a period");
}
```

- [ ] **Step 2: Run the integration test**

```bash
cargo test --test end_to_end
```

Expected: passes.

- [ ] **Step 3: Run the entire test suite**

```bash
cargo test
```

Expected: every test from Tasks 2–15 passes, plus the end-to-end test.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test(e2e): full pipeline integration test with scripted ASR + editor + mock platform"
```

---

## Self-Review

(Performed by the plan author after writing.)

**Spec coverage:** Every spec-defined component is covered by Tasks 1–17 except wizard, doctor, and extract (deferred to Plans 2–4 by design).

**Type consistency:** `Diff` (Task 4), `ChunkEvent`/`EndReason` (Task 5), `Asr` trait (Task 6), `Editor` trait (Task 7), `Platform` trait (Task 8), `Injector` (Task 14), and `Pipeline` (Task 15) all reference each other with stable signatures.

**Placeholder scan:** No "TBD", "TODO", or implementation-by-prose steps. All code blocks are complete enough to compile.

**Known caveats called out:**
- Naive resampler in `audio.rs` — adequate for v1, can swap to `rubato` later.
- Live `cpal` capture, `evdev` reads, `uinput` writes, and `arboard` Wayland behavior are tested via the manual checklist in Task 16, not unit tests.
- Windows hotkey + input tested manually on a Windows box (Task 16 checklist) — `cargo check --target x86_64-pc-windows-gnu` only verifies it compiles.
