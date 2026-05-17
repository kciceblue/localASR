# localASR `setup` Wizard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `localasr setup` — a GUI wizard (egui/eframe) that walks a first-time user through choosing a mic (with a live level meter), configuring the ASR and editor endpoints (with "Test" buttons), recording a push-to-talk hotkey, running an end-to-end speech test, and writing a complete `config.toml`.

**Architecture:** Single-window egui app driven by a `WizardStep` state machine. A background Tokio runtime handles async probes (ASR/editor HTTP calls, one-shot speech session); results flow back to the GUI via `oneshot::Receiver`s polled each frame. The mic level meter runs cpal capture on its own thread, writing a smoothed RMS into an `Arc<AtomicU32>` (f32 bits) that egui samples each frame. The `WizardState` struct accumulates all collected values and serializes to TOML on save.

**Tech Stack:** New deps — `eframe` 0.29 (egui 0.29 reexported). Existing — Tokio, cpal, the daemon/platform/doctor/config modules.

**Spec:** `docs/superpowers/specs/2026-05-17-localasr-design.md` (the "Subcommands" section, setup bullet: *"Multi-step wizard: 1) pick mic (with a 'speak now' level meter), 2) configure ASR endpoint URL/model/key, 3) configure editor endpoint URL/model/key, 4) record a hotkey by pressing it, 5) end-to-end test (record → transcribe → edit → paste), 6) Writes complete config.toml. One-stop setup."*).

**Prerequisites:** Plans 1 and 2 complete. This plan starts a NEW branch `feat/wizard` from current HEAD (Plan 2 end at `f6c3d3e`).

---

## File Structure

| File | Responsibility |
|---|---|
| `src/wizard/mod.rs` | `run()` entry: spawn tokio runtime, launch eframe |
| `src/wizard/state.rs` | `WizardState` (all collected values), `WizardStep` enum, `WizardState::to_config()` builder |
| `src/wizard/app.rs` | `eframe::App` impl: step router, runtime handle, pending-async polling |
| `src/wizard/runtime.rs` | Thin wrapper: spawn tokio runtime on a background thread, expose `Handle` |
| `src/wizard/level_meter.rs` | cpal capture thread + RMS smoother + `Arc<AtomicU32>` (f32 bits) |
| `src/wizard/steps/welcome.rs` | Intro screen |
| `src/wizard/steps/mic.rs` | Device picker + live level meter |
| `src/wizard/steps/asr.rs` | URL/key/model form + "Test" button |
| `src/wizard/steps/editor.rs` | Same shape, editor endpoint |
| `src/wizard/steps/hotkey.rs` | Capture egui key events → binding string |
| `src/wizard/steps/test.rs` | Spawn one-shot pipeline run, show transcript |
| `src/wizard/steps/save.rs` | Show review, write `config.toml` |
| `src/wizard/steps/mod.rs` | Re-exports |
| `src/main.rs` | Replace the `Cmd::Setup` `bail!` with `wizard::run()` |
| `src/lib.rs` | Add `pub mod wizard;` |

---

## Build Sequence Rationale

Tasks land bottom-up. The first 3 tasks build the GUI shell and runtime plumbing — they're the hardest to get right because of the async/GUI bridge. The remaining tasks add one step at a time; each step is independently runnable (you can see it by launching the wizard and clicking through prior steps).

The mic step's level meter (Task 4) is the most code in any single step — it needs a dedicated background thread plus per-frame polling. The hotkey-capture step (Task 7) is the trickiest UX-wise because egui's key events need translating into our binding-string format.

---

### Task 1: Wizard scaffold — deps, empty window, runtime

**Files:**
- Create: `src/wizard/mod.rs`
- Create: `src/wizard/state.rs`
- Create: `src/wizard/app.rs`
- Create: `src/wizard/runtime.rs`
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`

- [ ] **Step 1: Branch from current HEAD**

```bash
cd /home/kciceblue/HF/localASR
git checkout -b feat/wizard
```

- [ ] **Step 2: Add `eframe` to `[dependencies]` in `Cargo.toml`**

```toml
eframe = { version = "0.29", default-features = false, features = ["default_fonts", "wgpu"] }
```

(Disable `glow` default; use `wgpu` for consistent cross-platform rendering. Adapt to whatever the actual current eframe version is — if 0.29 isn't published, use the latest 0.x.)

- [ ] **Step 3: Write `src/wizard/runtime.rs`**

```rust
//! Spawn a Tokio runtime on a background thread and expose its handle to
//! the (sync) eframe GUI loop. Async work scheduled via `Handle::spawn`
//! runs on the background runtime; results return through any preferred
//! sync channel (`oneshot::Receiver`, `std::sync::mpsc::Receiver`, etc.).

use std::thread;
use tokio::runtime::{Handle, Runtime};

pub struct WizardRuntime {
    pub handle: Handle,
    _thread: thread::JoinHandle<()>,
}

impl WizardRuntime {
    pub fn start() -> anyhow::Result<Self> {
        let rt = Runtime::new()?;
        let handle = rt.handle().clone();
        // Move the runtime into a parking thread so it stays alive.
        let _thread = thread::Builder::new()
            .name("wizard-runtime".into())
            .spawn(move || {
                rt.block_on(async {
                    std::future::pending::<()>().await;
                });
            })?;
        Ok(Self { handle, _thread })
    }
}
```

- [ ] **Step 4: Write `src/wizard/state.rs` — minimal step enum**

```rust
//! Wizard state machine and collected values. Steps mutate this; on save it
//! serializes to a `Config`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Welcome,
    Mic,
    Asr,
    Editor,
    Hotkey,
    Test,
    Save,
    Done,
}

#[derive(Debug, Default, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    // Fields land in later tasks as their steps need them.
}

impl Default for WizardStep {
    fn default() -> Self { WizardStep::Welcome }
}

impl WizardState {
    pub fn new() -> Self { Self::default() }
}
```

- [ ] **Step 5: Write `src/wizard/app.rs`**

```rust
//! egui app shell. Currently shows a placeholder screen with the current step
//! name; real step rendering lands in Tasks 3+.

use crate::wizard::runtime::WizardRuntime;
use crate::wizard::state::{WizardState, WizardStep};
use eframe::egui;

pub struct WizardApp {
    pub state: WizardState,
    pub runtime: WizardRuntime,
}

impl WizardApp {
    pub fn new(runtime: WizardRuntime) -> Self {
        Self { state: WizardState::new(), runtime }
    }
}

impl eframe::App for WizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("localasr setup");
            ui.label(format!("current step: {:?}", self.state.step));
            ui.add_space(8.0);
            ui.label("(individual steps land in later tasks)");
        });
    }
}
```

- [ ] **Step 6: Write `src/wizard/mod.rs`**

```rust
//! `localasr setup` — interactive GUI wizard.

pub mod app;
pub mod runtime;
pub mod state;

use anyhow::Result;
use app::WizardApp;
use runtime::WizardRuntime;

pub fn run() -> Result<()> {
    let runtime = WizardRuntime::start()?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([720.0, 520.0])
            .with_title("localasr setup"),
        ..Default::default()
    };
    eframe::run_native(
        "localasr setup",
        options,
        Box::new(|_cc| Ok(Box::new(WizardApp::new(runtime)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;
    Ok(())
}
```

- [ ] **Step 7: Wire into `src/lib.rs`** — add `pub mod wizard;` in alphabetical position:

```rust
pub mod config;
pub mod daemon;
pub mod doctor;
pub mod platform;
pub mod terms;
pub mod wizard;
```

- [ ] **Step 8: Compile sanity**

```bash
cargo build
cargo test --lib
```

`cargo build` should compile (eframe pulls in wgpu + winit which is a substantial download — first build will take several minutes). `cargo test --lib` should still pass all 62 tests.

If `eframe` 0.29 doesn't exist yet, look at `cargo search eframe` for the latest 0.x and use that. If `Box::new(|_cc| Ok(...))` isn't the right closure signature for the version you pick, adapt (older eframe used `|_cc| Box::new(...)` with no Result wrapper).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(wizard): scaffold eframe app + tokio runtime bridge"
```

---

### Task 2: Wizard navigation — Welcome step + Next/Back

**Files:**
- Modify: `src/wizard/app.rs`
- Modify: `src/wizard/state.rs`
- Create: `src/wizard/steps/mod.rs`
- Create: `src/wizard/steps/welcome.rs`

- [ ] **Step 1: Write `src/wizard/steps/welcome.rs`**

```rust
use eframe::egui;
use crate::wizard::state::{WizardState, WizardStep};

pub fn render(state: &mut WizardState, ui: &mut egui::Ui) {
    ui.heading("welcome to localasr");
    ui.add_space(12.0);
    ui.label("this wizard configures your push-to-talk dictation setup:");
    ui.add_space(8.0);
    ui.label("  • pick a microphone");
    ui.label("  • point at an ASR endpoint (OpenAI-compatible)");
    ui.label("  • point at a chat/completions endpoint for polishing");
    ui.label("  • record a hotkey");
    ui.label("  • run an end-to-end test");
    ui.label("  • save your config.toml");
    ui.add_space(24.0);
    if ui.button("start").clicked() {
        state.step = WizardStep::Mic;
    }
}
```

- [ ] **Step 2: Write `src/wizard/steps/mod.rs`**

```rust
pub mod welcome;
// other steps added in later tasks
```

Also add step-navigation helpers used by every step. Append to the same file:

```rust
use eframe::egui;
use crate::wizard::state::{WizardState, WizardStep};

/// Renders a horizontal "Back  Next" footer. Caller decides whether `next` is
/// enabled (e.g., disabled until required fields are filled in).
/// Returns Some(WizardStep) if the user clicked a navigation button.
pub fn nav(ui: &mut egui::Ui, back: WizardStep, next: Option<WizardStep>) -> Option<WizardStep> {
    let mut clicked = None;
    ui.add_space(16.0);
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("back").clicked() {
            clicked = Some(back);
        }
        let next_enabled = next.is_some();
        let resp = ui.add_enabled(next_enabled, egui::Button::new("next"));
        if resp.clicked() {
            clicked = next;
        }
    });
    clicked
}
```

- [ ] **Step 3: Modify `src/wizard/app.rs` to route to step renderers**

```rust
use crate::wizard::runtime::WizardRuntime;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps;
use eframe::egui;

pub struct WizardApp {
    pub state: WizardState,
    pub runtime: WizardRuntime,
}

impl WizardApp {
    pub fn new(runtime: WizardRuntime) -> Self {
        Self { state: WizardState::new(), runtime }
    }
}

impl eframe::App for WizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.state.step {
                WizardStep::Welcome => steps::welcome::render(&mut self.state, ui),
                WizardStep::Done => {
                    ui.heading("done");
                    ui.label("close this window and run `localasr daemon`.");
                }
                _ => {
                    ui.heading(format!("{:?}", self.state.step));
                    ui.label("(this step lands in a later task)");
                    if let Some(s) = steps::nav(ui, WizardStep::Welcome, Some(self.state.step.next())) {
                        self.state.step = s;
                    }
                }
            }
        });
    }
}
```

- [ ] **Step 4: Add `WizardStep::next` helper in `src/wizard/state.rs`**

Append to `state.rs`:

```rust
impl WizardStep {
    pub fn next(self) -> WizardStep {
        match self {
            WizardStep::Welcome => WizardStep::Mic,
            WizardStep::Mic => WizardStep::Asr,
            WizardStep::Asr => WizardStep::Editor,
            WizardStep::Editor => WizardStep::Hotkey,
            WizardStep::Hotkey => WizardStep::Test,
            WizardStep::Test => WizardStep::Save,
            WizardStep::Save => WizardStep::Done,
            WizardStep::Done => WizardStep::Done,
        }
    }

    pub fn prev(self) -> WizardStep {
        match self {
            WizardStep::Welcome => WizardStep::Welcome,
            WizardStep::Mic => WizardStep::Welcome,
            WizardStep::Asr => WizardStep::Mic,
            WizardStep::Editor => WizardStep::Asr,
            WizardStep::Hotkey => WizardStep::Editor,
            WizardStep::Test => WizardStep::Hotkey,
            WizardStep::Save => WizardStep::Test,
            WizardStep::Done => WizardStep::Save,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn next_then_prev_returns_to_start() {
        let s = WizardStep::Welcome;
        assert_eq!(s.next().prev(), s);
    }

    #[test]
    fn done_is_terminal() {
        assert_eq!(WizardStep::Done.next(), WizardStep::Done);
    }

    #[test]
    fn welcome_prev_is_self() {
        assert_eq!(WizardStep::Welcome.prev(), WizardStep::Welcome);
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test --lib wizard::
```

Expected: 3 passing tests.

- [ ] **Step 6: Manual smoke test (optional, requires display)**

```bash
cargo run -- setup
```

(But we haven't wired `Cmd::Setup` yet — that's Task 9. For now skip the live test; the compile is enough.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(wizard): step machine + Welcome screen + Next/Back nav"
```

---

### Task 3: Mic step — device picker + level meter

**Files:**
- Create: `src/wizard/level_meter.rs`
- Create: `src/wizard/steps/mic.rs`
- Modify: `src/wizard/mod.rs` (export new module)
- Modify: `src/wizard/state.rs` (add `mic_device: Option<String>`)
- Modify: `src/wizard/steps/mod.rs` (export `mic`)
- Modify: `src/wizard/app.rs` (route `WizardStep::Mic` to `steps::mic::render`)

- [ ] **Step 1: Write `src/wizard/level_meter.rs`**

```rust
//! Spawns a cpal input stream on its own thread, computes a smoothed RMS over
//! incoming frames, and exposes it as an `Arc<AtomicU32>` (f32 bits) for the
//! egui thread to read at frame rate.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, Sender};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

pub struct LevelMeter {
    pub level: Arc<AtomicU32>,  // current RMS as f32 bits, 0.0..=1.0
    stop_tx: Sender<()>,
    _thread: thread::JoinHandle<()>,
}

impl LevelMeter {
    pub fn start(device_name: &str) -> Result<Self> {
        let level = Arc::new(AtomicU32::new(0));
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let device_name = device_name.to_string();
        let level_for_thread = level.clone();

        let _thread = thread::Builder::new().name("wizard-level-meter".into()).spawn(move || {
            if let Err(e) = run(device_name, level_for_thread, stop_rx) {
                tracing::warn!("level meter thread crashed: {e:?}");
            }
        })?;

        Ok(Self { level, stop_tx, _thread })
    }

    /// Current RMS, 0.0..=1.0.
    pub fn current(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

impl Drop for LevelMeter {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

fn run(device_name: String, level: Arc<AtomicU32>, stop_rx: crossbeam_channel::Receiver<()>) -> Result<()> {
    let host = cpal::default_host();
    let device = if device_name.is_empty() {
        host.default_input_device().context("no default input device")?
    } else {
        host.input_devices()?
            .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
            .with_context(|| format!("input device not found: {device_name}"))?
    };
    let config = device.default_input_config()?;
    let channels = config.channels() as usize;

    let lvl_for_cb = level.clone();
    let stream = match config.sample_format() {
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data: &[i16], _| {
                let rms = rms_i16(data, channels);
                let smoothed = smooth(f32::from_bits(lvl_for_cb.load(Ordering::Relaxed)), rms);
                lvl_for_cb.store(smoothed.to_bits(), Ordering::Relaxed);
            },
            |e| tracing::warn!("level meter stream error: {e}"),
            None,
        )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| {
                let rms = rms_f32(data, channels);
                let smoothed = smooth(f32::from_bits(lvl_for_cb.load(Ordering::Relaxed)), rms);
                lvl_for_cb.store(smoothed.to_bits(), Ordering::Relaxed);
            },
            |e| tracing::warn!("level meter stream error: {e}"),
            None,
        )?,
        _ => anyhow::bail!("unsupported sample format"),
    };

    stream.play()?;
    let _ = stop_rx.recv();
    drop(stream);
    Ok(())
}

fn rms_i16(data: &[i16], channels: usize) -> f32 {
    if data.is_empty() { return 0.0; }
    let mut sum = 0.0f32;
    for c in data.chunks_exact(channels) {
        let avg: f32 = c.iter().map(|&x| x as f32 / 32768.0).sum::<f32>() / channels as f32;
        sum += avg * avg;
    }
    let n = (data.len() / channels) as f32;
    (sum / n).sqrt().min(1.0)
}

fn rms_f32(data: &[f32], channels: usize) -> f32 {
    if data.is_empty() { return 0.0; }
    let mut sum = 0.0f32;
    for c in data.chunks_exact(channels) {
        let avg: f32 = c.iter().sum::<f32>() / channels as f32;
        sum += avg * avg;
    }
    let n = (data.len() / channels) as f32;
    (sum / n).sqrt().min(1.0)
}

/// One-pole smoothing toward the new level. Coefficient chosen so the meter
/// reacts within ~50ms but doesn't flicker every frame.
fn smooth(prev: f32, new: f32) -> f32 {
    const ALPHA: f32 = 0.3;
    prev * (1.0 - ALPHA) + new * ALPHA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_silence_is_zero() {
        assert_eq!(rms_i16(&vec![0i16; 1000], 1), 0.0);
    }

    #[test]
    fn rms_max_is_one() {
        let s: Vec<i16> = vec![i16::MAX; 1000];
        // i16::MAX as f32 / 32768.0 ≈ 0.99997; squared and averaged stays just under 1.0
        assert!((rms_i16(&s, 1) - 0.99997).abs() < 0.001);
    }

    #[test]
    fn smooth_converges() {
        let mut lvl = 0.0;
        for _ in 0..50 {
            lvl = smooth(lvl, 1.0);
        }
        assert!(lvl > 0.99);
    }
}
```

- [ ] **Step 2: Add `mic_device` field to `WizardState` in `src/wizard/state.rs`**

```rust
#[derive(Debug, Default, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub mic_device: String,  // empty = default
}
```

(remove `#[derive(Clone)]` if `LevelMeter` is added to the state — it's not; the meter lives on the app, not in state.)

- [ ] **Step 3: Write `src/wizard/steps/mic.rs`**

```rust
use eframe::egui;
use cpal::traits::{DeviceTrait, HostTrait};

use crate::wizard::level_meter::LevelMeter;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

/// Per-step UI state held by the app (so the LevelMeter survives across frames).
#[derive(Default)]
pub struct MicStepState {
    pub devices: Vec<String>,
    pub selected: String,
    pub meter: Option<LevelMeter>,
}

impl MicStepState {
    pub fn refresh_devices(&mut self) {
        self.devices.clear();
        let host = cpal::default_host();
        if let Ok(iter) = host.input_devices() {
            for d in iter {
                if let Ok(name) = d.name() {
                    self.devices.push(name);
                }
            }
        }
    }

    /// Start the level meter for the currently-selected device, replacing any prior.
    pub fn start_meter(&mut self) {
        self.meter = None; // drop any prior to release the device
        match LevelMeter::start(&self.selected) {
            Ok(m) => self.meter = Some(m),
            Err(e) => tracing::warn!("level meter start failed: {e:?}"),
        }
    }
}

pub fn render(
    state: &mut WizardState,
    mic: &mut MicStepState,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("pick a microphone");
    ui.add_space(8.0);

    if mic.devices.is_empty() {
        mic.refresh_devices();
    }

    if ui.button("rescan devices").clicked() {
        mic.refresh_devices();
    }

    ui.add_space(8.0);
    ui.label("device:");
    let prev_selected = mic.selected.clone();
    egui::ComboBox::from_id_source("mic_device")
        .selected_text(if mic.selected.is_empty() { "(system default)" } else { mic.selected.as_str() })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut mic.selected, String::new(), "(system default)");
            for d in &mic.devices {
                ui.selectable_value(&mut mic.selected, d.clone(), d);
            }
        });

    if mic.selected != prev_selected || mic.meter.is_none() {
        mic.start_meter();
    }

    ui.add_space(12.0);
    ui.label("speak — the bar should move:");
    let level = mic.meter.as_ref().map(|m| m.current()).unwrap_or(0.0);
    // Visual: a horizontal bar 0..=1
    let (rect, _) = ui.allocate_exact_size(egui::vec2(400.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let filled = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * level.clamp(0.0, 1.0), rect.height()),
    );
    painter.rect_filled(filled, 2.0, egui::Color32::from_rgb(80, 200, 120));
    ui.label(format!("level: {:.3}", level));

    // Save the selected device to wizard state so we don't lose it on back/forward.
    state.mic_device = mic.selected.clone();

    nav(ui, WizardStep::Welcome, Some(WizardStep::Asr))
}
```

- [ ] **Step 4: Wire `MicStepState` into `WizardApp`** in `src/wizard/app.rs`:

```rust
use crate::wizard::runtime::WizardRuntime;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps;
use crate::wizard::steps::mic::MicStepState;
use eframe::egui;

pub struct WizardApp {
    pub state: WizardState,
    pub runtime: WizardRuntime,
    pub mic: MicStepState,
}

impl WizardApp {
    pub fn new(runtime: WizardRuntime) -> Self {
        Self { state: WizardState::new(), runtime, mic: MicStepState::default() }
    }
}

impl eframe::App for WizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Force repaints so the level meter animates.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));

        egui::CentralPanel::default().show(ctx, |ui| {
            let nav_result = match self.state.step {
                WizardStep::Welcome => { steps::welcome::render(&mut self.state, ui); None }
                WizardStep::Mic => steps::mic::render(&mut self.state, &mut self.mic, ui),
                WizardStep::Done => {
                    ui.heading("done");
                    ui.label("close this window and run `localasr daemon`.");
                    None
                }
                _ => {
                    ui.heading(format!("{:?}", self.state.step));
                    ui.label("(this step lands in a later task)");
                    steps::nav(ui, self.state.step.prev(), Some(self.state.step.next()))
                }
            };
            if let Some(s) = nav_result {
                self.state.step = s;
                // When leaving the mic step, stop the level meter to release the device.
                if self.state.step != WizardStep::Mic {
                    self.mic.meter = None;
                }
            }
        });
    }
}
```

- [ ] **Step 5: Register modules**

In `src/wizard/mod.rs`, add `pub mod level_meter;`. In `src/wizard/steps/mod.rs`, add `pub mod mic;`.

- [ ] **Step 6: Run tests**

```bash
cargo test --lib wizard::
```

Expected: 3 wizard tests + 3 level-meter tests = 6 passing.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(wizard): mic step with device picker + live level meter"
```

---

### Task 4: ASR endpoint step with Test button

**Files:**
- Create: `src/wizard/steps/asr.rs`
- Modify: `src/wizard/state.rs` (add ASR fields)
- Modify: `src/wizard/steps/mod.rs` (export `asr`)
- Modify: `src/wizard/app.rs` (route + add per-step state for pending tests)

- [ ] **Step 1: Extend `WizardState` in `src/wizard/state.rs`**

Replace the struct with:

```rust
#[derive(Debug, Default, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub mic_device: String,

    // ASR endpoint
    pub asr_base_url: String,
    pub asr_api_key: String,
    pub asr_model: String,
    pub asr_test_result: Option<Result<(), String>>,

    // Editor endpoint
    pub editor_base_url: String,
    pub editor_api_key: String,
    pub editor_model: String,
    pub editor_test_result: Option<Result<(), String>>,

    // Hotkey
    pub hotkey_binding: String,

    // Test step
    pub test_transcript: Option<String>,
}
```

Update the default initializer to provide sensible defaults:

```rust
impl WizardState {
    pub fn new() -> Self {
        Self {
            asr_base_url: "https://api.openai.com/v1".into(),
            asr_model: "whisper-1".into(),
            editor_base_url: "https://api.openai.com/v1".into(),
            editor_model: "gpt-4o-mini".into(),
            hotkey_binding: "RightCtrl".into(),
            ..Default::default()
        }
    }
}
```

- [ ] **Step 2: Write `src/wizard/steps/asr.rs`**

```rust
use eframe::egui;
use tokio::sync::oneshot;
use tokio::runtime::Handle;

use crate::config::AsrConfig;
use crate::doctor::probes::asr_probe;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct AsrStepState {
    pub pending: Option<oneshot::Receiver<Result<(), String>>>,
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut AsrStepState,
    rt_handle: &Handle,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("ASR endpoint");
    ui.add_space(8.0);

    ui.label("base URL:");
    ui.text_edit_singleline(&mut state.asr_base_url);
    ui.add_space(4.0);
    ui.label("API key (use the literal key, or '${ENV_VAR}' to defer):");
    ui.text_edit_singleline(&mut state.asr_api_key);
    ui.add_space(4.0);
    ui.label("model:");
    ui.text_edit_singleline(&mut state.asr_model);

    ui.add_space(12.0);

    // Poll pending test result
    if let Some(rx) = step_state.pending.as_mut() {
        if let Ok(result) = rx.try_recv() {
            state.asr_test_result = Some(result);
            step_state.pending = None;
        }
    }

    let testing = step_state.pending.is_some();
    let test_btn = ui.add_enabled(!testing, egui::Button::new(if testing { "testing…" } else { "test" }));
    if test_btn.clicked() {
        let cfg = AsrConfig {
            base_url: state.asr_base_url.clone(),
            api_key: state.asr_api_key.clone(),
            model: state.asr_model.clone(),
            language: "".into(),
            timeout_ms: 10000,
        };
        let (tx, rx) = oneshot::channel();
        rt_handle.spawn(async move {
            let result = asr_probe(cfg).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        step_state.pending = Some(rx);
    }

    if let Some(r) = &state.asr_test_result {
        match r {
            Ok(()) => { ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "✓ endpoint responded OK"); }
            Err(e) => { ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("✗ {e}")); }
        }
    }

    let next_ok = state.asr_test_result.as_ref().map(|r| r.is_ok()).unwrap_or(false);
    nav(ui, WizardStep::Mic, if next_ok { Some(WizardStep::Editor) } else { None })
}
```

- [ ] **Step 3: Wire into app.rs and steps/mod.rs**

Add `pub mod asr;` to `src/wizard/steps/mod.rs`.

In `src/wizard/app.rs`:
- Add `pub asr: steps::asr::AsrStepState,` to `WizardApp`
- Initialize it in `new()` as `steps::asr::AsrStepState::default()`
- Add the route in `update()`:

```rust
WizardStep::Asr => steps::asr::render(&mut self.state, &mut self.asr, &self.runtime.handle, ui),
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib wizard::
```

Expected: still 6 passing tests (no new tests; ASR-probe is already covered in doctor tests).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(wizard): ASR endpoint step with async Test button"
```

---

### Task 5: Editor endpoint step

**Files:**
- Create: `src/wizard/steps/editor.rs`
- Modify: `src/wizard/steps/mod.rs`
- Modify: `src/wizard/app.rs`

Mirror Task 4's structure for the editor endpoint. The only differences:
- Uses `EditorConfig` and `editor_probe` from `crate::doctor::probes`
- Form fields write to `state.editor_*` instead of `state.asr_*`
- Test result stored in `state.editor_test_result`
- Back goes to `WizardStep::Asr`; Next goes to `WizardStep::Hotkey`

- [ ] **Step 1: Write `src/wizard/steps/editor.rs`**

```rust
use eframe::egui;
use tokio::sync::oneshot;
use tokio::runtime::Handle;

use crate::config::{EditorConfig, EditorPassConfig};
use crate::doctor::probes::editor_probe;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct EditorStepState {
    pub pending: Option<oneshot::Receiver<Result<(), String>>>,
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut EditorStepState,
    rt_handle: &Handle,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("editor endpoint");
    ui.add_space(8.0);
    ui.label("This endpoint polishes ASR output for grammar/jargon. Can be the same provider as ASR or different.");
    ui.add_space(8.0);

    ui.label("base URL:");
    ui.text_edit_singleline(&mut state.editor_base_url);
    ui.add_space(4.0);
    ui.label("API key:");
    ui.text_edit_singleline(&mut state.editor_api_key);
    ui.add_space(4.0);
    ui.label("model:");
    ui.text_edit_singleline(&mut state.editor_model);

    ui.add_space(12.0);

    if let Some(rx) = step_state.pending.as_mut() {
        if let Ok(result) = rx.try_recv() {
            state.editor_test_result = Some(result);
            step_state.pending = None;
        }
    }

    let testing = step_state.pending.is_some();
    let test_btn = ui.add_enabled(!testing, egui::Button::new(if testing { "testing…" } else { "test" }));
    if test_btn.clicked() {
        let cfg = EditorConfig {
            base_url: state.editor_base_url.clone(),
            api_key: state.editor_api_key.clone(),
            model: state.editor_model.clone(),
            light_temperature: 0.0,
            heavy_temperature: 0.2,
            light_timeout_ms: 5000,
            heavy_timeout_ms: 15000,
            light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
            heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
        };
        let (tx, rx) = oneshot::channel();
        rt_handle.spawn(async move {
            let result = editor_probe(cfg).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        step_state.pending = Some(rx);
    }

    if let Some(r) = &state.editor_test_result {
        match r {
            Ok(()) => { ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "✓ endpoint responded OK"); }
            Err(e) => { ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("✗ {e}")); }
        }
    }

    let next_ok = state.editor_test_result.as_ref().map(|r| r.is_ok()).unwrap_or(false);
    nav(ui, WizardStep::Asr, if next_ok { Some(WizardStep::Hotkey) } else { None })
}
```

- [ ] **Step 2: Wire it in**

Add `pub mod editor;` to `src/wizard/steps/mod.rs`.

In `src/wizard/app.rs`:
- Add `pub editor: steps::editor::EditorStepState,` to `WizardApp`
- Initialize in `new()`
- Add route:

```rust
WizardStep::Editor => steps::editor::render(&mut self.state, &mut self.editor, &self.runtime.handle, ui),
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test --lib wizard::
```

```bash
git add -A
git commit -m "feat(wizard): editor endpoint step"
```

---

### Task 6: Hotkey capture step

**Files:**
- Create: `src/wizard/steps/hotkey.rs`
- Modify: `src/wizard/steps/mod.rs`
- Modify: `src/wizard/app.rs`

- [ ] **Step 1: Write `src/wizard/steps/hotkey.rs`**

```rust
//! Hotkey-capture step. We use egui's own input events (focused-window key
//! events, not global) to record what the user pressed. The result is a
//! binding string compatible with the Linux/Windows hotkey parsers.

use eframe::egui;
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct HotkeyStepState {
    pub capturing: bool,
}

/// Map egui Key → the string our binding parser accepts. Returns None for keys
/// we don't support (e.g. arrow keys).
fn key_to_binding_name(k: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match k {
        Space => "Space",
        F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4",
        F5 => "F5", F6 => "F6", F7 => "F7", F8 => "F8",
        F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
        _ => return None,
    })
}

/// Build a binding string from the current egui modifier state + a non-modifier key.
fn build_chord(mods: egui::Modifiers, key: Option<&'static str>) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    if mods.ctrl { parts.push("Ctrl"); }
    if mods.shift { parts.push("Shift"); }
    if mods.alt { parts.push("Alt"); }
    if mods.mac_cmd || mods.command { parts.push("Meta"); }
    if let Some(k) = key { parts.push(k); }
    parts.join("+")
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut HotkeyStepState,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("hotkey");
    ui.add_space(8.0);
    ui.label("press the key (or chord) you want to hold for push-to-talk.");
    ui.label("supported keys: F1–F12, Space, plus modifiers Ctrl/Shift/Alt/Meta.");
    ui.label("the daemon also accepts single modifier keys (RightCtrl is the default).");
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("current binding:");
        ui.monospace(&state.hotkey_binding);
    });

    ui.add_space(8.0);
    if step_state.capturing {
        ui.colored_label(egui::Color32::from_rgb(220, 180, 80), "listening… press a key");
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    if let Some(name) = key_to_binding_name(*key) {
                        state.hotkey_binding = build_chord(*modifiers, Some(name));
                        step_state.capturing = false;
                        break;
                    }
                }
            }
        });
    } else if ui.button("record new binding").clicked() {
        step_state.capturing = true;
    }

    ui.add_space(8.0);
    ui.label("(or type it directly — the daemon's parser accepts the same syntax)");
    ui.text_edit_singleline(&mut state.hotkey_binding);

    nav(ui, WizardStep::Editor, Some(WizardStep::Test))
}
```

- [ ] **Step 2: Wire it in**

Add `pub mod hotkey;` to `src/wizard/steps/mod.rs`.

In `src/wizard/app.rs`:
- Add `pub hotkey: steps::hotkey::HotkeyStepState,` to `WizardApp`
- Initialize in `new()`
- Add route (note: this step needs `ctx` not just `ui` — pass it through):

```rust
WizardStep::Hotkey => steps::hotkey::render(&mut self.state, &mut self.hotkey, ctx, ui),
```

- [ ] **Step 3: Add tests for `build_chord`** in `src/wizard/steps/hotkey.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_only() {
        let mods = egui::Modifiers { ctrl: true, ..Default::default() };
        assert_eq!(build_chord(mods, None), "Ctrl");
    }

    #[test]
    fn chord_with_key() {
        let mods = egui::Modifiers { ctrl: true, alt: true, ..Default::default() };
        assert_eq!(build_chord(mods, Some("Space")), "Ctrl+Alt+Space");
    }

    #[test]
    fn key_only() {
        assert_eq!(build_chord(egui::Modifiers::default(), Some("F8")), "F8");
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib wizard::
```

Expected: 6 prior + 3 new = 9 wizard tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(wizard): hotkey capture step"
```

---

### Task 7: End-to-end test step

**Files:**
- Create: `src/wizard/steps/test.rs`
- Modify: `src/wizard/steps/mod.rs`
- Modify: `src/wizard/app.rs`

The test step runs a one-shot, pre-recorded session: capture for 5 seconds, transcribe, polish, show result. The user reads the transcript on-screen — no need to paste into a focused app at this stage.

- [ ] **Step 1: Write `src/wizard/steps/test.rs`**

```rust
//! End-to-end speech test. Captures audio for a fixed window, runs ASR + the
//! heavy editor pass, displays the transcript in-window.

use eframe::egui;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::config::{AsrConfig, EditorConfig, EditorPassConfig};
use crate::daemon::asr_client::{Asr, OpenAiAsr};
use crate::daemon::audio::AudioCapture;
use crate::daemon::editor_client::{Editor, OpenAiEditor};
use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct TestStepState {
    pub pending: Option<oneshot::Receiver<Result<String, String>>>,
}

const RECORD_SECONDS: u64 = 5;

pub fn render(
    state: &mut WizardState,
    step_state: &mut TestStepState,
    rt_handle: &Handle,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("end-to-end test");
    ui.add_space(8.0);
    ui.label(format!("click record, speak for {RECORD_SECONDS} seconds, then we'll show what was transcribed."));

    if let Some(rx) = step_state.pending.as_mut() {
        if let Ok(result) = rx.try_recv() {
            match result {
                Ok(t) => state.test_transcript = Some(t),
                Err(e) => state.test_transcript = Some(format!("(error: {e})")),
            }
            step_state.pending = None;
        }
    }

    let recording = step_state.pending.is_some();
    let label = if recording { "recording…" } else { "record" };
    if ui.add_enabled(!recording, egui::Button::new(label)).clicked() {
        let device = state.mic_device.clone();
        let asr_cfg = AsrConfig {
            base_url: state.asr_base_url.clone(),
            api_key: state.asr_api_key.clone(),
            model: state.asr_model.clone(),
            language: "".into(),
            timeout_ms: 30000,
        };
        let editor_cfg = EditorConfig {
            base_url: state.editor_base_url.clone(),
            api_key: state.editor_api_key.clone(),
            model: state.editor_model.clone(),
            light_temperature: 0.0, heavy_temperature: 0.2,
            light_timeout_ms: 5000, heavy_timeout_ms: 20000,
            light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
            heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
        };
        let (tx, rx) = oneshot::channel();
        rt_handle.spawn(async move {
            let result = run_test(device, asr_cfg, editor_cfg).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        step_state.pending = Some(rx);
    }

    ui.add_space(12.0);
    if let Some(t) = &state.test_transcript {
        ui.label("transcript:");
        ui.add(egui::TextEdit::multiline(&mut t.clone()).desired_rows(4).desired_width(f32::INFINITY));
    }

    let next_ok = state.test_transcript.as_ref().map(|t| !t.starts_with("(error")).unwrap_or(false);
    nav(ui, WizardStep::Hotkey, if next_ok { Some(WizardStep::Save) } else { None })
}

async fn run_test(device: String, asr: AsrConfig, editor: EditorConfig) -> anyhow::Result<String> {
    // Capture for RECORD_SECONDS, accumulating all frames.
    let cap = AudioCapture::start(&device, 16000)?;
    let frames_rx = cap.frames.clone();
    let collect_handle = tokio::task::spawn_blocking(move || -> Vec<i16> {
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(RECORD_SECONDS);
        while std::time::Instant::now() < deadline {
            match frames_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(buf) => all.extend(buf),
                Err(_) => {}
            }
        }
        all
    });
    let samples = collect_handle.await?;
    cap.stop();

    if samples.is_empty() {
        anyhow::bail!("no audio captured (device silent?)");
    }

    let asr_client = OpenAiAsr::new(asr)?;
    let raw = asr_client.transcribe(&samples).await?;
    if raw.trim().is_empty() {
        return Ok("(silence — no speech detected)".into());
    }

    let editor_client = OpenAiEditor::new(editor)?;
    let polished = editor_client.polish_heavy(&raw, None).await?;
    Ok(polished)
}
```

- [ ] **Step 2: Wire it in**

Add `pub mod test;` to `src/wizard/steps/mod.rs`.

In `src/wizard/app.rs`:
- Add `pub test: steps::test::TestStepState,` to `WizardApp`
- Initialize in `new()`
- Route:

```rust
WizardStep::Test => steps::test::render(&mut self.state, &mut self.test, &self.runtime.handle, ui),
```

- [ ] **Step 3: Compile + commit (no unit tests — purely manual)**

```bash
cargo build
cargo test --lib wizard::
```

Expected: still 9 wizard tests pass.

```bash
git add -A
git commit -m "feat(wizard): end-to-end speech test step"
```

---

### Task 8: Save step

**Files:**
- Create: `src/wizard/steps/save.rs`
- Modify: `src/wizard/state.rs` (add `to_config()` method)
- Modify: `src/wizard/steps/mod.rs`
- Modify: `src/wizard/app.rs`

- [ ] **Step 1: Add `WizardState::to_config()` and a test** in `src/wizard/state.rs`

Append (after the existing impls):

```rust
use crate::config::{
    AsrConfig, AudioConfig, Config, EditorConfig, EditorPassConfig, HotkeyConfig,
    InjectionConfig, TermsConfig, VadBackend, VadConfig,
};

impl WizardState {
    /// Build a complete Config from the values the wizard collected.
    /// Fields not surfaced in the UI get sensible defaults.
    pub fn to_config(&self) -> Config {
        Config {
            hotkey: HotkeyConfig { binding: self.hotkey_binding.clone() },
            audio: AudioConfig { device: self.mic_device.clone(), sample_rate: 16000 },
            vad: VadConfig { backend: VadBackend::Silero, min_silence_ms: 400, max_chunk_ms: 5000 },
            asr: AsrConfig {
                base_url: self.asr_base_url.clone(),
                api_key: self.asr_api_key.clone(),
                model: self.asr_model.clone(),
                language: "".into(),
                timeout_ms: 15000,
            },
            editor: EditorConfig {
                base_url: self.editor_base_url.clone(),
                api_key: self.editor_api_key.clone(),
                model: self.editor_model.clone(),
                light_temperature: 0.0, heavy_temperature: 0.2,
                light_timeout_ms: 4000, heavy_timeout_ms: 15000,
                light: EditorPassConfig { enabled: true, context_chunks: 3, system_prompt: "".into() },
                heavy: EditorPassConfig { enabled: true, context_chunks: 0, system_prompt: "".into() },
            },
            terms: TermsConfig { enabled: false, path: "~/.config/localasr/terms.toml".into() },
            injection: InjectionConfig {
                mode: "clipboard_paste".into(),
                paste_shortcut: "Ctrl+V".into(),
                restore_delay_ms: 100,
            },
        }
    }
}

#[cfg(test)]
mod cfg_tests {
    use super::*;

    #[test]
    fn to_config_round_trips_through_toml() {
        let mut s = WizardState::new();
        s.asr_api_key = "sk-test".into();
        s.editor_api_key = "sk-test".into();
        s.hotkey_binding = "F8".into();
        let cfg = s.to_config();
        let serialized = toml::to_string(&cfg).unwrap();
        let parsed: crate::config::Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.hotkey.binding, "F8");
        assert_eq!(parsed.asr.model, "whisper-1");
        assert_eq!(parsed.editor.model, "gpt-4o-mini");
    }
}
```

- [ ] **Step 2: Write `src/wizard/steps/save.rs`**

```rust
use eframe::egui;
use anyhow::Result;
use std::path::PathBuf;

use crate::wizard::state::{WizardState, WizardStep};
use crate::wizard::steps::nav;

#[derive(Default)]
pub struct SaveStepState {
    pub last_save_result: Option<Result<PathBuf, String>>,
}

pub fn render(
    state: &mut WizardState,
    step_state: &mut SaveStepState,
    ui: &mut egui::Ui,
) -> Option<WizardStep> {
    ui.heading("save");
    ui.add_space(8.0);

    let cfg = state.to_config();
    let preview = toml::to_string_pretty(&cfg).unwrap_or_else(|e| format!("(serialization error: {e})"));
    ui.label("config.toml preview:");
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut preview.clone())
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
        });

    ui.add_space(8.0);
    if ui.button("save and finish").clicked() {
        match save_config(&cfg) {
            Ok(path) => {
                step_state.last_save_result = Some(Ok(path));
            }
            Err(e) => {
                step_state.last_save_result = Some(Err(format!("{e:#}")));
            }
        }
    }

    if let Some(r) = &step_state.last_save_result {
        match r {
            Ok(path) => {
                ui.colored_label(egui::Color32::from_rgb(80, 200, 120), format!("✓ saved to {}", path.display()));
                ui.label("close this window and run: localasr daemon");
            }
            Err(e) => {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("✗ {e}"));
            }
        }
    }

    let next_ok = step_state.last_save_result.as_ref().map(|r| r.is_ok()).unwrap_or(false);
    nav(ui, WizardStep::Test, if next_ok { Some(WizardStep::Done) } else { None })
}

fn save_config(cfg: &crate::config::Config) -> Result<PathBuf> {
    let path = crate::config::default_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, text)?;
    Ok(path)
}
```

- [ ] **Step 3: Wire it in**

Add `pub mod save;` to `src/wizard/steps/mod.rs`.

In `src/wizard/app.rs`:
- Add `pub save: steps::save::SaveStepState,` to `WizardApp`
- Initialize in `new()`
- Route:

```rust
WizardStep::Save => steps::save::render(&mut self.state, &mut self.save, ui),
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib wizard::
```

Expected: 9 prior + 1 new (to_config round-trip) = 10 wizard tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(wizard): save step writes config.toml to default path"
```

---

### Task 9: CLI wiring + README

**Files:**
- Modify: `src/main.rs`
- Modify: `README.md`

- [ ] **Step 1: Replace the `Cmd::Setup` arm in `src/main.rs`**

```rust
Cmd::Setup => localasr::wizard::run(),
```

(`localasr::wizard::run` returns `anyhow::Result<()>`, which matches `main`'s return type.)

- [ ] **Step 2: Update `README.md`**

In the "## Diagnostics" section (or a new "## Setup" section just before it), add:

```markdown
## Setup wizard

First-time users should run:

```
localasr setup
```

This launches a GUI wizard (egui) that walks through:

1. Pick a microphone (with a live "speak now" level meter)
2. Configure the ASR endpoint URL/key/model and verify with a test request
3. Configure the editor endpoint and verify
4. Record your push-to-talk hotkey
5. Record a 5-second speech sample to verify the full pipeline
6. Save your `config.toml` to the platform's default config path

The wizard writes a complete `config.toml` to `~/.config/localasr/config.toml` (Linux) or `%APPDATA%\localasr\config.toml` (Windows).
```

- [ ] **Step 3: Verify build + full test suite**

```bash
cargo build
cargo test
```

Expected: all 63 + 10 wizard tests = 73 total (62 existing lib + 10 wizard lib + 1 e2e).

- [ ] **Step 4: Manual smoke test (optional, requires a display)**

```bash
cargo run -- setup
```

Walk through the wizard. With no real endpoint, the ASR and editor "Test" buttons will fail with connection errors — that's expected; the wizard correctly gates "next" until tests succeed. You can type bogus values to get past mic and hotkey, but ASR/editor will block. To smoke-test without a server, you can temporarily change the gating in `asr.rs` / `editor.rs` (revert before commit).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(wizard): wire \`localasr setup\` into CLI + README"
```

---

## Self-Review

(Performed by the plan author after writing.)

**Spec coverage:**
- *"pick mic (with a 'speak now' level meter)"* — Task 3
- *"configure ASR endpoint URL/model/key"* + test button — Task 4
- *"configure editor endpoint URL/model/key"* + test button — Task 5
- *"record a hotkey by pressing it"* — Task 6
- *"end-to-end test (record → transcribe → edit → paste)"* — Task 7 (record + transcribe + edit; "paste" is interpreted as "display in the wizard window" since the wizard isn't a paste target, and the goal is verification not actual injection)
- *"Writes complete config.toml"* — Task 8

**Placeholder scan:** No TBD/TODO. eframe version is `0.29` with a fallback note to use whatever's current. The egui version-dependent API calls (e.g., `Box::new(|_cc| Ok(Box::new(...)))`) include adapt notes.

**Type consistency:** `WizardState` fields are added incrementally across Tasks 3, 4, 5, 6, 7, 8 — verify each new task's field additions don't conflict with prior. Step-state structs (`MicStepState`, `AsrStepState`, `EditorStepState`, `HotkeyStepState`, `TestStepState`, `SaveStepState`) live on `WizardApp`, not in `WizardState`, because they hold non-cloneable things like `LevelMeter` and `oneshot::Receiver`.

**Scope check:** Single coherent feature (one GUI wizard). 9 tasks of varying size. Tasks 3 and 7 are the largest; the rest are mechanical. The bridge between sync GUI and async tokio (oneshot::Receiver polling) is the key pattern repeated across tasks 4, 5, 7.

**Known limitations:**
- Spec says "end-to-end test (record → transcribe → edit → paste)" but Task 7 omits the paste step — the wizard window isn't a paste target, so showing the transcript in the wizard is the sensible substitute.
- The level meter (Task 3) and end-to-end test (Task 7) require real audio hardware to verify. No unit tests for those paths.
- The hotkey-capture step (Task 6) only captures keys the *focused egui window* receives, not global. The user can also type the binding directly into the text field as a fallback.
- egui 0.29 (or whichever current version) is a large dep — the wizard binary will be substantially larger than the daemon binary. The plan does NOT split into two binaries (per Plan 1's decision to keep one binary).
- Linux Wayland support depends on whether the chosen winit backend in eframe negotiates wgpu/Vulkan correctly. Manual verification needed on Wayland (GNOME + KDE) and X11.
