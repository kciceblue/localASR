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
use tokio::sync::{mpsc, oneshot};
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

    // Tracks an in-flight session: (task handle, graceful-stop sender).
    let mut current: Option<(JoinHandle<()>, oneshot::Sender<()>)> = None;

    while let Some(ev) = hotkey_rx.recv().await {
        match ev {
            HotkeyEvent::Press => {
                // If a session is already running, abort it immediately (re-press = abort).
                if let Some((handle, _stop_tx)) = current.take() {
                    handle.abort();
                    let _ = handle.await; // ignore JoinError from abort
                }
                let pipeline = Pipeline {
                    cfg: cfg.clone(),
                    asr: asr.clone(),
                    editor: editor.clone(),
                    injector: injector.clone(),
                    terms: terms.clone(),
                };
                let cfg2 = cfg.clone();
                let (stop_tx, stop_rx) = oneshot::channel::<()>();
                let handle = tokio::spawn(async move {
                    if let Err(e) = run_session(pipeline, cfg2, stop_rx).await {
                        tracing::warn!("session ended with error: {e:?}");
                    }
                });
                current = Some((handle, stop_tx));
            }
            HotkeyEvent::Release => {
                if let Some((handle, stop_tx)) = current.take() {
                    // Graceful stop: signal audio to stop, then wait for the task to
                    // naturally drain (which lets the pipeline run finalize / heavy pass).
                    let _ = stop_tx.send(());
                    let _ = handle.await;
                }
            }
        }
    }
    Ok(())
}

async fn run_session(pipeline: Pipeline, cfg: Config, stop_rx: oneshot::Receiver<()>) -> Result<()> {
    // Whisper expects 16 kHz mono; sample_rate in config is currently ignored.
    let capture = AudioCapture::start(&cfg.audio.device, 16000)?;
    let (chunk_tx, chunk_rx) = mpsc::channel::<crate::daemon::vad::ChunkEvent>(64);

    let cfg_clone = cfg.clone();
    let frames = capture.frames.clone();
    let chunk_tx_clone = chunk_tx.clone();
    let vad_handle = tokio::task::spawn_blocking(move || -> Result<()> {
        // Whisper expects 16 kHz mono; sample_rate in config is currently ignored.
        let mut vad = crate::daemon::vad::make(&cfg_clone.vad, 16000)?;
        let mut chunker = Chunker::new(16000, &cfg_clone.vad);
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

    // Drop chunk_tx so chunk_rx closes when the VAD task drops its clone.
    // This is a defensive measure: if SessionClosed isn't emitted for any reason,
    // the channel still closes naturally once the VAD task finishes.
    drop(chunk_tx);

    // Drive pipeline + watch for stop signal in parallel.
    // - If stop_rx fires first: signal capture to stop, then await pipeline to natural finish.
    // - If pipeline finishes first (shouldn't normally happen during PTT): cleanup.
    let pipeline_fut = pipeline.run(chunk_rx);
    tokio::pin!(pipeline_fut);

    let pipeline_result = tokio::select! {
        _ = stop_rx => {
            // Graceful shutdown: stop audio capture, then let the pipeline drain naturally.
            // Audio thread exits → frames channel closes → VAD frames.iter() ends →
            // chunker.close() emits SessionClosed → Pipeline::finalize (heavy editor pass) runs.
            capture.stop();
            (&mut pipeline_fut).await
        }
        result = &mut pipeline_fut => {
            capture.stop();
            result
        }
    };

    let _ = vad_handle.await;
    pipeline_result
}
