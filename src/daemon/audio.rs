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
