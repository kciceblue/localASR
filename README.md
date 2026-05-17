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
