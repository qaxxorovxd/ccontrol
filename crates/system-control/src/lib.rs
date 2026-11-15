//! Native system-control adapters.
//!
//! All calls are blocking (COM / D-Bus / subprocess); the agent runs them on
//! `spawn_blocking`. Each adapter reports *why* something is unavailable so
//! the capability manifest can stay honest.

use ccontrol_protocol::{MediaKey, Telemetry, VolumeState};

#[cfg(target_os = "linux")]
mod linux;
mod telemetry;
#[cfg(windows)]
mod windows;

pub use telemetry::TelemetrySampler;

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("platform call failed: {0}")]
    Platform(String),
}

/// Master output volume of the default playback endpoint.
pub trait VolumeControl: Send + Sync {
    fn get(&self) -> Result<VolumeState, ControlError>;
    fn set_level(&self, level: f32) -> Result<VolumeState, ControlError>;
    fn set_muted(&self, muted: bool) -> Result<VolumeState, ControlError>;
}

pub trait MediaControl: Send + Sync {
    fn send(&self, key: MediaKey) -> Result<(), ControlError>;
}

/// Platform-default volume adapter.
pub fn volume() -> Box<dyn VolumeControl> {
    #[cfg(windows)]
    {
        Box::new(windows::CoreAudioVolume)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::CliVolume)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(Unsupported)
    }
}

/// Platform-default media adapter.
pub fn media() -> Box<dyn MediaControl> {
    #[cfg(windows)]
    {
        Box::new(windows::MediaKeys)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::Mpris)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(Unsupported)
    }
}

#[allow(dead_code)]
struct Unsupported;

impl VolumeControl for Unsupported {
    fn get(&self) -> Result<VolumeState, ControlError> {
        Err(ControlError::Unsupported(
            "no volume adapter for this OS".into(),
        ))
    }
    fn set_level(&self, _: f32) -> Result<VolumeState, ControlError> {
        self.get()
    }
    fn set_muted(&self, _: bool) -> Result<VolumeState, ControlError> {
        self.get()
    }
}

impl MediaControl for Unsupported {
    fn send(&self, _: MediaKey) -> Result<(), ControlError> {
        Err(ControlError::Unsupported(
            "no media adapter for this OS".into(),
        ))
    }
}

pub(crate) fn check_level(level: f32) -> Result<f32, ControlError> {
    if level.is_finite() && (0.0..=1.0).contains(&level) {
        Ok(level)
    } else {
        Err(ControlError::Invalid(
            "level must be within 0.0..=1.0".into(),
        ))
    }
}

/// Telemetry type re-exported for convenience.
pub type TelemetrySnapshot = Telemetry;
