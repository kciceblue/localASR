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

## Term-database extraction

To help the heavy editor pass disambiguate jargon and names, you can
auto-generate a `terms.toml` from an existing corpus (your codebase, docs,
or any text folder):

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
may take a while — each chunk is logged to stderr as it's processed.
Per-chunk failures are logged but don't abort the run.

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
- `hotkey listener` — validate the binding string parses and the listener constructor returns without error (does not currently verify device permissions; see issue tracker for follow-up)

Exits 0 on all-pass, 1 if any check fails.

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
