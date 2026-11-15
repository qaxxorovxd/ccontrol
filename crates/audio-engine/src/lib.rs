//! ccontrol audio engine.
//!
//! Pipeline (one route):
//!
//! ```text
//! source agent                                   destination agent
//! capture (10 ms f32 frames) -> Opus encode ->   UDP recv -> AEAD open -> replay check
//!   AEAD seal (ChaCha20-Poly1305, per-route key)   -> jitter buffer -> Opus decode/PLC
//!   -> UDP send  <----- receiver reports ------    -> render (WASAPI / PipeWire)
//! ```
//!
//! Audio threads are plain OS threads (no async) to keep latency predictable.
//! All formats are fixed at 48 kHz stereo f32; devices are asked to convert.

pub mod codec;
pub mod device;
pub mod jitter;
pub mod packet;
pub mod pipeline;
pub mod pwdump;
pub mod tone;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

pub use device::*;
pub use pipeline::{ReceiverHandle, SenderHandle};

/// Samples per channel in one 10 ms frame at 48 kHz.
pub const FRAME_SAMPLES_PER_CH: usize = 480;
pub const CHANNELS: usize = 2;
/// Interleaved samples per frame.
pub const FRAME_LEN: usize = FRAME_SAMPLES_PER_CH * CHANNELS;

pub type Frame = Vec<f32>;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("device: {0}")]
    Device(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("network: {0}")]
    Net(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

/// Peak absolute sample value of a frame.
pub fn peak(frame: &[f32]) -> f32 {
    frame.iter().fold(0.0f32, |m, s| m.max(s.abs())).min(1.0)
}
