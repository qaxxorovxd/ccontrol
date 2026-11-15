//! WASAPI backend.
//!
//! * Inventory: render endpoints + audio sessions of the default endpoint
//!   (`IAudioSessionManager2`), each with PID, state, peak, mute.
//! * Capture: endpoint loopback (all Windows 10 builds) and process loopback
//!   (`ActivateAudioInterfaceAsync` + `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`,
//!   Windows build 20348+ only).
//! * Render: shared-mode `IAudioClient`, 48 kHz stereo float with
//!   `AUTOCONVERTPCM` so the engine never resamples itself on Windows.
//! * Local silencing: `ISimpleAudioVolume::SetMute` on the app's sessions.

mod loopback;
mod render;
mod sessions;

use crate::device::{AudioSink, AudioSource, AudioSystem, ToneSource};
use crate::AudioError;
use ccontrol_protocol::{AudioInventory, CaptureKind, RouteSource};
use windows::Win32::Media::Audio::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0};
use windows::Win32::Media::KernelStreaming::{
    SPEAKER_FRONT_LEFT, SPEAKER_FRONT_RIGHT, WAVE_FORMAT_EXTENSIBLE,
};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Minimum Windows build with process-loopback capture.
pub const PROCESS_LOOPBACK_BUILD: u32 = 20348;

pub(crate) fn win(e: windows::core::Error) -> AudioError {
    AudioError::Device(e.to_string())
}

pub(crate) struct Com(bool);
impl Com {
    pub(crate) fn init() -> Self {
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

/// 48 kHz, stereo, 32-bit float.
pub(crate) fn format_48k_stereo_f32() -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 32,
        },
        dwChannelMask: SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
        SubFormat: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
    }
}

pub fn windows_build() -> u32 {
    sysinfo::System::kernel_version()
        .and_then(|k| k.rsplit('.').next().and_then(|b| b.trim().parse().ok()))
        .unwrap_or(0)
}

pub struct WasapiSystem {
    build: u32,
}

impl Default for WasapiSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl WasapiSystem {
    pub fn new() -> Self {
        Self {
            build: windows_build(),
        }
    }

    fn process_capture(&self) -> Result<(), String> {
        if self.build >= PROCESS_LOOPBACK_BUILD {
            Ok(())
        } else {
            Err(format!(
                "Per-app capture needs Windows build {PROCESS_LOOPBACK_BUILD} or newer (this PC: {}). Whole-output capture is available.",
                self.build
            ))
        }
    }
}

impl AudioSystem for WasapiSystem {
    fn inventory(&self) -> Result<AudioInventory, AudioError> {
        let _com = Com::init();
        let endpoints = sessions::endpoints()?;
        let pc = self.process_capture();
        let mut list = sessions::sessions()?;
        for s in &mut list {
            s.process_capture = pc.is_ok() && !s.system_sounds;
            s.process_capture_reason = pc.clone().err();
            s.can_silence_local = s.process_capture;
        }
        Ok(AudioInventory {
            can_play: !endpoints.is_empty(),
            endpoint_capture: !endpoints.is_empty(),
            endpoints,
            sessions: list,
            process_capture: pc.is_ok(),
            process_capture_reason: pc.err(),
        })
    }

    fn open_source(&self, src: &RouteSource) -> Result<Box<dyn AudioSource>, AudioError> {
        match src {
            RouteSource::TestTone => Ok(Box::new(ToneSource::new(0.15))),
            RouteSource::Endpoint => Ok(Box::new(loopback::LoopbackSource::endpoint())),
            RouteSource::Session { pid, .. } => {
                self.process_capture().map_err(AudioError::Unsupported)?;
                Ok(Box::new(loopback::LoopbackSource::process(*pid)))
            }
        }
    }

    fn open_sink(&self, endpoint_id: Option<&str>) -> Result<Box<dyn AudioSink>, AudioError> {
        let _com = Com::init();
        let name = sessions::endpoint_name(endpoint_id)?;
        Ok(Box::new(render::RenderSink::new(
            endpoint_id.map(str::to_owned),
            name,
        )))
    }

    fn session_mutes(&self, pid: u32) -> Result<Vec<(String, bool)>, AudioError> {
        let _com = Com::init();
        sessions::mutes_of(pid)
    }

    fn set_session_mute(&self, session_id: &str, muted: bool) -> Result<bool, AudioError> {
        let _com = Com::init();
        sessions::set_mute(session_id, muted)
    }

    fn silence_local_supported(&self, kind: CaptureKind) -> Result<(), String> {
        match kind {
            CaptureKind::Process => self.process_capture(),
            CaptureKind::Endpoint => Err("Whole-output capture includes local playback; silencing it would silence the stream. Local playback is kept.".into()),
            CaptureKind::TestTone => Err("Nothing to silence for a test tone".into()),
        }
    }
}
