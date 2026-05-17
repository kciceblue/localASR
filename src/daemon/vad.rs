//! Voice-activity-driven chunking.
//!
//! The `Vad` trait answers "is this 30ms frame speech?" — implemented by silero
//! (default, neural) and webrtc (fallback, classical). The `Chunker` state machine
//! consumes a stream of (frame, is_speech) and emits chunk-boundary events.

use crate::config::schema::VadConfig;

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
    use crate::config::schema::VadConfig;

    fn cfg(min_silence_ms: u32, max_chunk_ms: u32) -> VadConfig {
        VadConfig {
            backend: crate::config::schema::VadBackend::Silero,
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

// webrtc_vad::Vad wraps a raw C pointer but is used exclusively through &mut self,
// so it is safe to send across threads.
unsafe impl Send for WebrtcVad {}

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
    use crate::config::schema::VadBackend;
    Ok(match cfg.backend {
        VadBackend::Silero => Box::new(SileroVad::new(sample_rate)?),
        VadBackend::Webrtc => Box::new(WebrtcVad::new(sample_rate)),
    })
}
