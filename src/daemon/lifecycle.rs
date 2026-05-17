//! The top-level daemon loop: subscribe to hotkey events, manage sessions,
//! handle abort-on-re-press.

use crate::config::Config;
use crate::daemon::asr_client::{Asr, OpenAiAsr};
use crate::daemon::audio::AudioCapture;
use crate::daemon::editor_client::{Editor, OpenAiEditor};
use crate::daemon::injector::Injector;
use crate::daemon::pipeline::Pipeline;
use crate::daemon::vad::Chunker;
use crate::platform::{HotkeyEvent, Platform};
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
