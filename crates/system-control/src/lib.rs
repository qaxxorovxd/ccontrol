//! Native system-control adapters.
//!
//! All calls are blocking (COM / D-Bus / subprocess); the agent runs them on
//! `spawn_blocking`. Each adapter reports *why* something is unavailable so
//! the capability manifest can stay honest.

use ccontrol_protocol::{AppWindow, MediaKey, MediaStatus, Telemetry, VolumeState};

#[cfg(target_os = "linux")]
mod linux;
pub mod processes;
mod telemetry;
#[cfg(windows)]
mod windows;

pub use processes::ProcessManager;
pub use telemetry::TelemetrySampler;

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("platform call failed: {0}")]
    Platform(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
}

/// Master output volume of the default playback endpoint.
pub trait VolumeControl: Send + Sync {
    fn get(&self) -> Result<VolumeState, ControlError>;
    fn set_level(&self, level: f32) -> Result<VolumeState, ControlError>;
    fn set_muted(&self, muted: bool) -> Result<VolumeState, ControlError>;
}

pub trait MediaControl: Send + Sync {
    fn send(&self, key: MediaKey) -> Result<(), ControlError>;
    /// Now playing; `MediaStatus::none()` when no session exists.
    fn status(&self) -> Result<MediaStatus, ControlError>;
}

/// Top-level application windows (not processes).
pub trait WindowControl: Send + Sync {
    fn list(&self) -> Result<Vec<AppWindow>, ControlError>;
    /// Politely ask one window to close (WM_CLOSE / _NET_CLOSE_WINDOW).
    fn close(&self, window_id: u64, pid: u32) -> Result<(), ControlError>;
    /// Ask every window of `pid` to close; returns how many were asked.
    fn close_all_of(&self, pid: u32) -> Result<usize, ControlError>;
}

/// Platform-default window adapter.
pub fn windows_adapter() -> Box<dyn WindowControl> {
    #[cfg(windows)]
    {
        Box::new(windows::apps::Win32Windows)
    }
    #[cfg(not(windows))]
    {
        Box::new(Unsupported)
    }
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
        Box::new(windows::smtc::WindowsMedia)
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
    fn status(&self) -> Result<MediaStatus, ControlError> {
        self.send(MediaKey::Stop).map(|_| MediaStatus::none())
    }
}

impl WindowControl for Unsupported {
    fn list(&self) -> Result<Vec<AppWindow>, ControlError> {
        Err(ControlError::Unsupported(if cfg!(target_os = "linux") {
            "Window listing is not implemented on Linux yet (X11 EWMH planned; Wayland has no generic window API)".into()
        } else {
            "Window listing is not available on this OS".into()
        }))
    }
    fn close(&self, _: u64, _: u32) -> Result<(), ControlError> {
        self.list().map(|_| ())
    }
    fn close_all_of(&self, _: u32) -> Result<usize, ControlError> {
        self.list().map(|_| 0)
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
