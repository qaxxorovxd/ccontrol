//! Audio routing protocol types (v0.3).
//!
//! A route moves one source (an app's audio session, a whole output device,
//! or a test tone) from a *source agent* to a *sink* on a destination agent.
//! The controller orchestrates: it prepares the sink, then asks the source to
//! start. The source agent owns the route state machine, its write-ahead
//! journal and its lease; the destination owns the sink and its lease.

use serde::{Deserialize, Serialize};

/// Audio format on the wire: 48 kHz, stereo, 10 ms Opus frames.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;
pub const AUDIO_FRAME_MS: u32 = 10;
/// Default lease; controllers renew every `lease/3`.
pub const DEFAULT_LEASE_SECS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioEndpoint {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    /// Only this application's audio (Windows process loopback / PipeWire stream).
    Process,
    /// Everything the default output device plays.
    Endpoint,
    /// Generated 440 Hz test signal (route verification).
    TestTone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSession {
    /// Stable for the session's lifetime (OS session instance / node id).
    pub session_id: String,
    pub pid: u32,
    pub app_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Currently producing sound (not just open).
    pub active: bool,
    pub peak: f32,
    pub muted: bool,
    pub volume: f32,
    pub system_sounds: bool,
    /// Whether this one app can be captured on this computer.
    pub process_capture: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_capture_reason: Option<String>,
    /// Whether local playback can be silenced reversibly while routing.
    pub can_silence_local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioInventory {
    pub endpoints: Vec<AudioEndpoint>,
    pub sessions: Vec<AudioSession>,
    /// Whole-output capture available (Windows endpoint loopback / PipeWire monitor).
    pub endpoint_capture: bool,
    pub process_capture: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_capture_reason: Option<String>,
    /// Can act as a destination (render a received stream).
    pub can_play: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteSource {
    /// One application's session(s), identified by process id.
    Session {
        pid: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_name: Option<String>,
    },
    /// The whole default output device.
    Endpoint,
    TestTone,
}

/// Source-side route state machine (docs/architecture/RECOVERY.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteState {
    Validating,
    StartingStream,
    VerifyingPlayback,
    ApplyingReversibleSourceChange,
    Active,
    Stopping,
    RestoringSource,
    RollingBack,
    Idle,
    RecoveryRequired,
}

impl RouteState {
    pub fn is_terminal(self) -> bool {
        matches!(self, RouteState::Idle | RouteState::RecoveryRequired)
    }
    /// States in which a reversible local change may be in effect.
    pub fn may_have_source_change(self) -> bool {
        matches!(
            self,
            RouteState::ApplyingReversibleSourceChange
                | RouteState::Active
                | RouteState::Stopping
                | RouteState::RestoringSource
                | RouteState::RollingBack
        )
    }
}

/// Truthful outcome of stop/rollback/recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOutcome {
    /// Original local playback state was restored.
    Restored,
    /// Nothing had been changed locally.
    NotNeeded,
    /// The user changed the setting meanwhile; it was left as they set it.
    LeftUserChange,
    /// Some sessions were restored, others no longer exist.
    Partial,
    /// Restoration failed; the user must fix it (e.g. unmute the app).
    ManualActionRequired,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamStats {
    pub packets: u64,
    pub lost: u64,
    pub late: u64,
    pub duplicates: u64,
    pub concealed_frames: u64,
    pub frames_played: u64,
    pub underruns: u64,
    pub jitter_ms: f32,
    pub buffer_ms: f32,
    /// Network round trip measured with receiver reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f32>,
    /// rtt/2 + jitter buffer + device buffer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f32>,
    pub bitrate_kbps: f32,
    /// Peak level of the audio (0..1) over the last report interval.
    pub peak: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteInfo {
    pub route_id: String,
    pub state: RouteState,
    pub source: RouteSource,
    pub capture: CaptureKind,
    pub dest: String,
    pub owner: String,
    pub silence_local_requested: bool,
    pub local_silenced: bool,
    pub lease_remaining_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RecoveryOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Sender view (with receiver-reported loss/peak/played frames).
    pub stats: StreamStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkState {
    Waiting,
    Playing,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SinkInfo {
    pub route_id: String,
    pub state: SinkState,
    pub udp_port: u16,
    pub endpoint_name: String,
    pub owner: String,
    pub lease_remaining_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub stats: StreamStats,
}

/// Validate a controller-chosen route id (opaque, ≤ 64 chars, URL-safe).
pub fn valid_route_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Validate a hex-encoded 32-byte stream key.
pub fn valid_stream_key(k: &str) -> bool {
    k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_shapes() {
        assert_eq!(
            serde_json::to_string(&RouteSource::Session {
                pid: 7,
                app_name: None
            })
            .unwrap(),
            r#"{"kind":"session","pid":7}"#
        );
        assert_eq!(
            serde_json::to_string(&RouteSource::TestTone).unwrap(),
            r#"{"kind":"test_tone"}"#
        );
    }

    #[test]
    fn validators() {
        assert!(valid_route_id("r-1_a"));
        assert!(!valid_route_id("../x"));
        assert!(!valid_route_id(""));
        assert!(valid_stream_key(&"ab".repeat(32)));
        assert!(!valid_stream_key("zz"));
    }

    #[test]
    fn states() {
        assert!(RouteState::Active.may_have_source_change());
        assert!(!RouteState::StartingStream.may_have_source_change());
        assert!(RouteState::RecoveryRequired.is_terminal());
    }
}
