//! Device abstraction: platform audio systems, sources and sinks.

use crate::tone::Tone;
use crate::{AudioError, Frame};
use ccontrol_protocol::{AudioInventory, CaptureKind, RouteSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Produces 10 ms 48 kHz stereo frames in real time.
pub trait AudioSource: Send {
    fn kind(&self) -> CaptureKind;
    /// Blocks until the next frame; `Ok(None)` once `stop` is set.
    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, AudioError>;
}

/// Renders frames pulled from `next` until `stop` is set.
pub trait AudioSink: Send {
    fn name(&self) -> String;
    fn run(&mut self, next: &mut dyn FnMut() -> Frame, stop: &AtomicBool)
        -> Result<(), AudioError>;
    /// Device-side buffering, for the latency estimate.
    fn device_latency_ms(&self) -> f32;
}

/// Platform audio: inventory, capture, render and reversible per-app silencing
/// (the "source change" of a route).
pub trait AudioSystem: Send + Sync {
    fn inventory(&self) -> Result<AudioInventory, AudioError>;
    fn open_source(&self, src: &RouteSource) -> Result<Box<dyn AudioSource>, AudioError>;
    fn open_sink(&self, endpoint_id: Option<&str>) -> Result<Box<dyn AudioSink>, AudioError>;
    /// Current mute state of every session belonging to `pid`.
    fn session_mutes(&self, pid: u32) -> Result<Vec<(String, bool)>, AudioError>;
    /// Set mute on one session instance; returns false if it no longer exists.
    fn set_session_mute(&self, session_id: &str, muted: bool) -> Result<bool, AudioError>;
    /// Whether silencing a session locally keeps it audible in our capture.
    fn silence_local_supported(&self, kind: CaptureKind) -> Result<(), String>;
}

/// Platform default.
pub fn native() -> Arc<dyn AudioSystem> {
    #[cfg(windows)]
    {
        Arc::new(crate::windows::WasapiSystem::new())
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(crate::linux::PipeWireSystem)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Arc::new(NullSystem)
    }
}

/// Paced test tone.
pub struct ToneSource {
    tone: Tone,
    next_deadline: Option<Instant>,
}

impl ToneSource {
    pub fn new(gain: f32) -> Self {
        Self {
            tone: Tone::new(440.0, gain),
            next_deadline: None,
        }
    }
}

impl AudioSource for ToneSource {
    fn kind(&self) -> CaptureKind {
        CaptureKind::TestTone
    }
    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, AudioError> {
        let deadline = self.next_deadline.get_or_insert_with(Instant::now);
        let now = Instant::now();
        if *deadline > now {
            std::thread::sleep(*deadline - now);
        }
        *deadline += Duration::from_millis(10);
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        Ok(Some(self.tone.next_frame()))
    }
}

/// Paced sink that keeps frames in memory (tests, headless destinations).
pub struct MemorySink {
    pub frames: Arc<Mutex<Vec<f32>>>,
    pub peaks: Arc<Mutex<Vec<f32>>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self {
            frames: Arc::default(),
            peaks: Arc::default(),
        }
    }
}

impl Default for MemorySink {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSink for MemorySink {
    fn name(&self) -> String {
        "memory".into()
    }
    fn run(
        &mut self,
        next: &mut dyn FnMut() -> Frame,
        stop: &AtomicBool,
    ) -> Result<(), AudioError> {
        let mut deadline = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            let f = next();
            self.peaks.lock().unwrap().push(crate::peak(&f));
            let mut all = self.frames.lock().unwrap();
            if all.len() < 48_000 * 2 * 30 {
                all.extend_from_slice(&f);
            }
            drop(all);
            deadline += Duration::from_millis(10);
            let now = Instant::now();
            if deadline > now {
                std::thread::sleep(deadline - now);
            }
        }
        Ok(())
    }
    fn device_latency_ms(&self) -> f32 {
        0.0
    }
}

/// No audio system (unsupported OS / tests).
pub struct NullSystem;

impl AudioSystem for NullSystem {
    fn inventory(&self) -> Result<AudioInventory, AudioError> {
        Err(AudioError::Unsupported("no audio system on this OS".into()))
    }
    fn open_source(&self, src: &RouteSource) -> Result<Box<dyn AudioSource>, AudioError> {
        match src {
            RouteSource::TestTone => Ok(Box::new(ToneSource::new(0.2))),
            _ => Err(AudioError::Unsupported(
                "no audio capture on this OS".into(),
            )),
        }
    }
    fn open_sink(&self, _: Option<&str>) -> Result<Box<dyn AudioSink>, AudioError> {
        Err(AudioError::Unsupported("no audio output on this OS".into()))
    }
    fn session_mutes(&self, _: u32) -> Result<Vec<(String, bool)>, AudioError> {
        Ok(vec![])
    }
    fn set_session_mute(&self, _: &str, _: bool) -> Result<bool, AudioError> {
        Ok(false)
    }
    fn silence_local_supported(&self, _: CaptureKind) -> Result<(), String> {
        Err("not supported".into())
    }
}
