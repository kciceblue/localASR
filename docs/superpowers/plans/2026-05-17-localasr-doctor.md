# localASR `doctor` Subcommand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `localasr doctor` subcommand that runs non-interactive diagnostic probes (config, mic, ASR endpoint, editor endpoint, hotkey, paste/clipboard) and reports PASS/FAIL per check. Exits 0 if all pass, 1 if any fail.

**Architecture:** A small `doctor` module with a `Check` abstraction (name + async closure returning `Result<()>`) and a sequential runner that prints results. Each probe is a standalone function exercising the real platform/client through the same code paths the daemon uses. Probes that talk to HTTP endpoints are unit-tested with `wiremock`; the orchestration is tested with fake `Check`s; platform/audio probes are exercised live when the user runs `doctor`.

**Tech Stack:** Same as Plan 1 — Rust, Tokio, reqwest, cpal, arboard/evdev/uinput on Linux, `windows` crate on Windows. New: nothing new.

**Spec:** `docs/superpowers/specs/2026-05-17-localasr-design.md` (the "Subcommands" section: *"`localasr doctor` — non-interactive diagnostics: probe mic, both endpoints, hotkey capture, paste mechanism. Prints pass/fail per check and exits."*).

**Prerequisites:** Plan 1 complete on branch `feat/core-daemon`. This plan starts a NEW branch from current HEAD.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/doctor/mod.rs` | `Check` struct, `run_all()` orchestrator, output formatter |
| `src/doctor/probes.rs` | Six probe functions: `config_probe`, `mic_probe`, `asr_probe`, `editor_probe`, `hotkey_probe`, `paste_probe` |
| `src/main.rs` | Replace the `Cmd::Doctor` `bail!` with a real dispatch into `doctor::run_all()` |
| `src/lib.rs` | Add `pub mod doctor;` |

---

## Build Sequence Rationale

Tasks land bottom-up: first the orchestration scaffold (unit-testable with fake checks), then each individual probe (tested via wiremock or trivial), finally the main.rs wiring.

Probes that need real hardware (`mic_probe`, `hotkey_probe`, `paste_probe`) ship with minimal-but-honest implementations that try the cheapest plausible call and report failure if it errors. They're exercised by the user when they run `doctor`; no synthetic unit tests for those.

---

### Task 1: Start branch and add `doctor` module scaffold

**Files:**
- Create: `src/doctor/mod.rs`
- Create: `src/doctor/probes.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Branch from current HEAD**

```bash
cd /home/kciceblue/HF/localASR
git checkout -b feat/doctor
```

- [ ] **Step 2: Write `src/doctor/mod.rs`**

```rust
//! `localasr doctor` — non-interactive diagnostic probes.
//!
//! A `Check` is a named async operation that returns `Result<()>`. The runner
//! executes them in order, prints `PASS` or `FAIL: <msg>` per check, and
//! returns the count of failures (so the CLI can set the exit code).

pub mod probes;

use anyhow::Result;
use futures::future::BoxFuture;

pub struct Check {
    pub name: &'static str,
    pub run: Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>,
}

impl Check {
    pub fn new<F, Fut>(name: &'static str, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        use futures::FutureExt;
        Check { name, run: Box::new(move || f().boxed()) }
    }
}

/// Run all checks sequentially. Prints results to stdout.
/// Returns the number of failures.
pub async fn run_all(checks: Vec<Check>) -> usize {
    let mut failures = 0;
    for check in checks {
        print!("  {} ... ", check.name);
        // Flush so the trailing dots show before the network/audio probe runs.
        use std::io::Write;
        let _ = std::io::stdout().flush();
        match (check.run)().await {
            Ok(()) => println!("PASS"),
            Err(e) => {
                println!("FAIL: {e:#}");
                failures += 1;
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_pass_returns_zero() {
        let checks = vec![
            Check::new("check_a", || async { Ok(()) }),
            Check::new("check_b", || async { Ok(()) }),
        ];
        assert_eq!(run_all(checks).await, 0);
    }

    #[tokio::test]
    async fn one_failure_counted() {
        let checks = vec![
            Check::new("ok", || async { Ok(()) }),
            Check::new("bad", || async { anyhow::bail!("nope") }),
            Check::new("ok2", || async { Ok(()) }),
        ];
        assert_eq!(run_all(checks).await, 1);
    }

    #[tokio::test]
    async fn all_fail_counted() {
        let checks = vec![
            Check::new("a", || async { anyhow::bail!("x") }),
            Check::new("b", || async { anyhow::bail!("y") }),
        ];
        assert_eq!(run_all(checks).await, 2);
    }
}
```

- [ ] **Step 3: Write empty `src/doctor/probes.rs`**

```rust
//! Individual diagnostic probes. Each probe is an async fn returning `Result<()>`.
//! Implementations land in Tasks 2–6.
```

- [ ] **Step 4: Add `futures = "0.3"` to `[dependencies]` in `Cargo.toml`**

Locate `[dependencies]` and append:
```toml
futures = "0.3"
```

(We need `futures::FutureExt::boxed` and `BoxFuture`. Tokio doesn't re-export these.)

- [ ] **Step 5: Add `pub mod doctor;` to `src/lib.rs`**

The current file (after Plan 1) is:
```rust
pub mod config;
pub mod daemon;
pub mod platform;
pub mod terms;
```

Make it:
```rust
pub mod config;
pub mod daemon;
pub mod doctor;
pub mod platform;
pub mod terms;
```

- [ ] **Step 6: Run tests**

```bash
cargo test --lib doctor::
```

Expected: 3 passing tests (the orchestrator tests).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(doctor): Check abstraction + sequential runner with tests"
```

---

### Task 2: `config_probe` — loads config from disk

**Files:**
- Modify: `src/doctor/probes.rs`

- [ ] **Step 1: Add `config_probe` and its test**

Append to `src/doctor/probes.rs`:

```rust
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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
```

Note: the top-of-file `//! Individual diagnostic probes...` comment from Task 1 stays. Just append below it. The `Path` import in the use statement is added even though only `PathBuf` is used in this probe — `Path` will be used by tests later. Actually scratch that, `Path` is not used; drop the unused import. Use only `use std::path::PathBuf;` and `use anyhow::{Context, Result};`.

- [ ] **Step 2: Run tests**

```bash
cargo test --lib doctor::probes
```

Expected: 3 passing tests.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(doctor): config_probe verifies config loads"
```

---

### Task 3: `asr_probe` — round-trip to the ASR endpoint

**Files:**
- Modify: `src/doctor/probes.rs`

- [ ] **Step 1: Add `asr_probe` and tests**

Append to `src/doctor/probes.rs`:

```rust
use crate::config::AsrConfig;
use crate::daemon::asr_client::{Asr, OpenAiAsr};

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
```

- [ ] **Step 2: Run tests**

```bash
cargo test --lib doctor::probes
```

Expected: 5 passing tests (3 from Task 2 + 2 new).

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(doctor): asr_probe round-trips a 0.5s silence sample"
```

---

### Task 4: `editor_probe` — round-trip to the editor endpoint

**Files:**
- Modify: `src/doctor/probes.rs`

- [ ] **Step 1: Add `editor_probe` and tests**

Append to `src/doctor/probes.rs`:

```rust
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
```

- [ ] **Step 2: Run tests**

```bash
cargo test --lib doctor::probes
```

Expected: 8 passing tests (5 from prior tasks + 3 new).

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(doctor): editor_probe verifies non-empty completion"
```

---

### Task 5: `mic_probe` — opens AudioCapture and waits for one frame

**Files:**
- Modify: `src/doctor/probes.rs`

- [ ] **Step 1: Add `mic_probe`**

Append to `src/doctor/probes.rs`:

```rust
use crate::daemon::audio::AudioCapture;
use std::time::Duration;

/// Probe: open the configured audio device and verify at least one frame
/// arrives within 2 seconds. Reports failure if the device can't be opened
/// or if it stays silent (no frames at all — usually means the device is busy
/// or the user has no input device).
pub async fn mic_probe(device_name: &str) -> Result<()> {
    let device_name = device_name.to_string();
    // AudioCapture::start is sync but spawns a thread; the cpal stream pumps
    // frames via crossbeam-channel. We poll the receiver from a blocking task.
    tokio::task::spawn_blocking(move || -> Result<()> {
        let cap = AudioCapture::start(&device_name, 16000)
            .context("opening audio device")?;
        let got = cap.frames.recv_timeout(Duration::from_secs(2));
        cap.stop();
        got.map(|_| ()).context("no audio frames within 2s (device silent or busy?)")
    })
    .await
    .context("mic probe task panicked")?
}
```

Note: there's no unit test for this probe. Live device testing happens when `localasr doctor` is run by the user. The `cargo test` suite will continue to skip this code path.

- [ ] **Step 2: Confirm it compiles**

```bash
cargo build
cargo test --lib doctor::probes
```

Expected: still 8 passing tests, no compile errors. New code is reachable but unexercised.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(doctor): mic_probe opens audio device and waits for first frame"
```

---

### Task 6: `paste_probe` and `hotkey_probe` — open platform devices

**Files:**
- Modify: `src/doctor/probes.rs`

- [ ] **Step 1: Add `paste_probe` and `hotkey_probe`**

Append to `src/doctor/probes.rs`:

```rust
use crate::platform::{ClipboardSnapshot, Platform};
use std::sync::Arc;

/// Probe: read and write the clipboard via the configured platform. This
/// catches Linux-Wayland permission issues and Windows `OpenClipboard` errors.
pub async fn paste_probe(platform: Arc<dyn Platform>) -> Result<()> {
    let original = platform.read_clipboard().await
        .context("reading clipboard (is the display server reachable?)")?;
    let probe_value = ClipboardSnapshot(Some("localasr-doctor-probe".to_string()));
    platform.write_clipboard(&probe_value).await
        .context("writing clipboard")?;
    platform.write_clipboard(&original).await
        .context("restoring clipboard")?;
    Ok(())
}

/// Probe: try to start a hotkey listener for a sentinel binding ("F24" — a key
/// that exists in evdev / windows VK tables but is almost never present on
/// real keyboards). Success means the listener thread started without
/// erroring (uinput permissions OK on Linux, hook installation OK on
/// Windows). The receiver is dropped immediately — we don't wait for events.
pub async fn hotkey_probe(platform: Arc<dyn Platform>) -> Result<()> {
    let _rx = platform.hotkey_stream("F24")
        .context("starting hotkey listener (check /dev/input permissions on Linux)")?;
    // _rx is dropped here, stopping the listener thread on next iteration.
    Ok(())
}

#[cfg(test)]
mod platform_probe_tests {
    use super::*;
    use crate::platform::mock::MockPlatform;

    #[tokio::test]
    async fn paste_probe_passes_with_mock() {
        let p = MockPlatform::new();
        assert!(paste_probe(p).await.is_ok());
    }

    #[tokio::test]
    async fn hotkey_probe_passes_with_mock() {
        let p = MockPlatform::new();
        assert!(hotkey_probe(p).await.is_ok());
    }
}
```

Note: these tests use `MockPlatform`, which trivially succeeds. The real value of these probes is detecting permission / device errors when invoked against `LinuxPlatform` or `WindowsPlatform` at runtime.

- [ ] **Step 2: There's an issue — `hotkey_stream` consumes `Arc<Self>` (`self: Arc<Self>`), not `&Arc<Self>`. Verify the call works**

Look at `src/platform/mod.rs` — `Platform::hotkey_stream` is:
```rust
fn hotkey_stream(self: Arc<Self>, binding: &str) -> Result<mpsc::Receiver<HotkeyEvent>>;
```

So `platform.hotkey_stream("F24")` actually moves the Arc. That's fine for `paste_probe` (which takes `Arc<dyn Platform>` by value and calls `read_clipboard(&self)`-style methods), but `hotkey_probe` takes `Arc<dyn Platform>` by value and calls `hotkey_stream` which consumes it. That's also fine — the function owns it.

But wait, if both `paste_probe` and `hotkey_probe` are called in sequence and each receives the Arc, the caller (`run_all`) needs to give each its own clone. We'll handle that in Task 7 (wiring). For now the probe signatures are correct.

- [ ] **Step 3: Run tests**

```bash
cargo test --lib doctor::probes
```

Expected: 10 passing tests (8 from prior + 2 new).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(doctor): paste_probe and hotkey_probe exercise the platform layer"
```

---

### Task 7: Wire `localasr doctor` into the CLI

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace the `Cmd::Doctor` arm in `src/main.rs`**

The current line:
```rust
Cmd::Doctor => anyhow::bail!("`localasr doctor` lands in Plan 2"),
```

becomes:
```rust
Cmd::Doctor => run_doctor().await,
```

- [ ] **Step 2: Add the `run_doctor` function**

After `run_daemon`, append:

```rust
async fn run_doctor() -> anyhow::Result<()> {
    use localasr::doctor::{run_all, Check};

    let config_path = localasr::config::default_config_path()?;
    let config_path_for_check = config_path.clone();

    // Load config once if present so the later checks can be built from it.
    let cfg_opt = if config_path.exists() {
        Some(localasr::config::load(&config_path)?)
    } else {
        None
    };

    let mut checks: Vec<Check> = Vec::new();

    checks.push(Check::new("config", move || async move {
        localasr::doctor::probes::config_probe(config_path_for_check).await
    }));

    if let Some(cfg) = cfg_opt {
        let asr_cfg = cfg.asr.clone();
        checks.push(Check::new("asr endpoint", move || async move {
            localasr::doctor::probes::asr_probe(asr_cfg).await
        }));

        let editor_cfg = cfg.editor.clone();
        checks.push(Check::new("editor endpoint", move || async move {
            localasr::doctor::probes::editor_probe(editor_cfg).await
        }));

        let device = cfg.audio.device.clone();
        checks.push(Check::new("microphone", move || async move {
            localasr::doctor::probes::mic_probe(&device).await
        }));

        // Build the platform once, share via Arc into each probe.
        #[cfg(target_os = "linux")]
        let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::linux::LinuxPlatform::new());
        #[cfg(target_os = "windows")]
        let platform: Arc<dyn localasr::platform::Platform> = Arc::new(localasr::platform::windows::WindowsPlatform::new());

        let p1 = platform.clone();
        checks.push(Check::new("clipboard", move || async move {
            localasr::doctor::probes::paste_probe(p1).await
        }));

        let p2 = platform.clone();
        checks.push(Check::new("hotkey listener", move || async move {
            localasr::doctor::probes::hotkey_probe(p2).await
        }));
    } else {
        eprintln!("(config missing — skipping endpoint/audio/platform checks)");
    }

    println!("running localasr diagnostics:");
    let failures = run_all(checks).await;
    if failures == 0 {
        println!("\nall checks passed");
        Ok(())
    } else {
        anyhow::bail!("{failures} check(s) failed")
    }
}
```

- [ ] **Step 3: Verify it compiles and runs (with no config — most checks skipped)**

```bash
cargo build
# Remove any existing config (or use a temp HOME if you prefer):
rm -f $HOME/.config/localasr/config.toml
cargo run -- doctor
```

Expected output:
```
running localasr diagnostics:
(config missing — skipping endpoint/audio/platform checks)
  config ... FAIL: no config file at /home/.../config.toml
Error: 1 check(s) failed
```

Process exits with non-zero.

- [ ] **Step 4: Verify full suite still passes**

```bash
cargo test
```

Expected: 49 unit tests + 10 doctor tests + 1 e2e = 60 tests passing.

- [ ] **Step 5: Update `README.md` to mention the `doctor` subcommand**

Find the "Manual verification checklist (Plan 1)" section in README.md, and BEFORE that section, add:

```markdown
## Diagnostics

Run `localasr doctor` after editing your config to verify all components work:

```
localasr doctor
```

Probes (in order):
- `config` — the TOML file exists and parses
- `asr endpoint` — round-trip a 0.5s silence sample to `/v1/audio/transcriptions`
- `editor endpoint` — ask the editor for a one-word completion
- `microphone` — open the configured input device and wait for the first frame (up to 2s)
- `clipboard` — read and restore the clipboard via the configured platform
- `hotkey listener` — start a sentinel-binding listener (checks /dev/input permissions on Linux)

Exits 0 on all-pass, 1 if any check fails.
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(doctor): wire `localasr doctor` into CLI + README"
```

---

## Self-Review

(Performed by the plan author after writing.)

**Spec coverage:** The spec line *"`localasr doctor` — non-interactive diagnostics: probe mic, both endpoints, hotkey capture, paste mechanism. Prints pass/fail per check and exits."* is fully covered:
- mic → Task 5
- ASR endpoint → Task 3
- editor endpoint → Task 4
- hotkey capture → Task 6
- paste mechanism → Task 6
- pass/fail per check + exit code → Task 1 + Task 7
- Config validation is a bonus probe (Task 2) since it's the first thing to fail anyway.

**Placeholder scan:** No TBD/TODO/etc. All code blocks compile-ready except for the `futures` import added in Task 1 Step 4 — that's an explicit dep addition.

**Type consistency:** All probe signatures are concrete (`PathBuf`, `AsrConfig`, `EditorConfig`, `&str`, `Arc<dyn Platform>`). The `Check::new` generic signature accepts any `FnOnce() -> Future<Output = Result<()>>` and is consistent across Tasks 1–6.

**Scope check:** Single coherent subsystem (a CLI subcommand). 7 tasks, ~15 minutes each. Fits in one plan.

**Known limitations:**
- `mic_probe`, `hotkey_probe`, and the live `paste_probe` cannot be unit-tested without hardware / a real display server. The MockPlatform-based tests for the latter two are smoke checks of the wiring only; they do NOT validate that the Linux/Windows platform code works on real hardware. This is acceptable because the same code paths are exercised by the daemon itself and by the README manual checklist.
- The `config` probe in Task 7 only checks the *default* config path. A `--config <path>` flag is not added in this plan — keeps scope tight.
