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
