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
    pub level: Arc<AtomicU32>, // current RMS as f32 bits, 0.0..=1.0
    stop_tx: Sender<()>,
    _thread: thread::JoinHandle<()>,
}

impl LevelMeter {
    pub fn start(device_name: &str) -> Result<Self> {
        let level = Arc::new(AtomicU32::new(0));
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let device_name = device_name.to_string();
        let level_for_thread = level.clone();

        let _thread = thread::Builder::new()
            .name("wizard-level-meter".into())
            .spawn(move || {
                if let Err(e) = run(device_name, level_for_thread, stop_rx) {
                    tracing::warn!("level meter thread crashed: {e:?}");
                }
            })?;

        Ok(Self {
            level,
            stop_tx,
            _thread,
        })
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

fn run(
    device_name: String,
    level: Arc<AtomicU32>,
    stop_rx: crossbeam_channel::Receiver<()>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = if device_name.is_empty() {
        host.default_input_device()
            .context("no default input device")?
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
                let smoothed =
                    smooth(f32::from_bits(lvl_for_cb.load(Ordering::Relaxed)), rms);
                lvl_for_cb.store(smoothed.to_bits(), Ordering::Relaxed);
            },
            |e| tracing::warn!("level meter stream error: {e}"),
            None,
        )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| {
                let rms = rms_f32(data, channels);
                let smoothed =
                    smooth(f32::from_bits(lvl_for_cb.load(Ordering::Relaxed)), rms);
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
    if data.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for c in data.chunks_exact(channels) {
        let avg: f32 =
            c.iter().map(|&x| x as f32 / 32768.0).sum::<f32>() / channels as f32;
        sum += avg * avg;
    }
    let n = (data.len() / channels) as f32;
    (sum / n).sqrt().min(1.0)
}

fn rms_f32(data: &[f32], channels: usize) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
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
