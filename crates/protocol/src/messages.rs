use crate::audio::{AudioInventory, RouteInfo, RouteSource, SinkInfo};
use crate::{CapabilityManifest, Scope};
use serde::{Deserialize, Serialize};

/// Every message exchanged on the control channel.
///
/// Session order:
/// 1. client `Hello` -> server `HelloAck { paired }`.
/// 2. If not paired: `PairCommit` -> `PairNonce` -> `PairReveal` ->
///    `PairAwaitingApproval` -> `PairResult`. On acceptance the same
///    connection continues as an authorized session.
/// 3. Authorized: `Request`/`Response`, server `Event`s, `Ping`/`Pong`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello(Hello),
    HelloAck(HelloAck),
    PairCommit {
        /// hex(SHA-256("ccontrol-commit-v1" || client_nonce))
        commitment: String,
        requested_scopes: Vec<Scope>,
    },
    PairNonce {
        /// hex, 32 random bytes chosen by the server after seeing the commitment.
        nonce: String,
    },
    PairReveal {
        /// hex, 32 random bytes; must hash to the earlier commitment.
        nonce: String,
    },
    PairAwaitingApproval {
        timeout_secs: u32,
    },
    PairResult {
        accepted: bool,
        granted_scopes: Vec<Scope>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Request {
        id: u64,
        #[serde(flatten)]
        action: Action,
    },
    Response {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<ActionResult>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ProtocolError>,
    },
    Event {
        #[serde(flatten)]
        event: AgentEvent,
    },
    Ping {
        seq: u64,
    },
    Pong {
        seq: u64,
    },
    /// Fatal error; the sender closes the connection afterwards.
    Error {
        error: ProtocolError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub min_protocol_version: u32,
    /// Fingerprint the client believes it presented (hex SHA-256 of its SPKI).
    pub device_id: String,
    /// Fingerprint the client observed for the server; must match the server's own.
    pub peer_device_id: String,
    pub device_name: String,
    /// android | windows | linux
    pub platform: String,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol_version: u32,
    pub device_id: String,
    pub device_name: String,
    pub paired: bool,
    pub granted_scopes: Vec<Scope>,
    /// Only sent to paired controllers holding `device.read`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<CapabilityManifest>,
}

/// Typed logical actions. Android never sends platform shell strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum Action {
    #[serde(rename = "device.info")]
    DeviceInfo,
    #[serde(rename = "telemetry.get")]
    TelemetryGet,
    #[serde(rename = "volume.get")]
    VolumeGet,
    #[serde(rename = "volume.set")]
    VolumeSet {
        /// Scalar 0.0..=1.0
        level: f32,
    },
    #[serde(rename = "volume.mute")]
    VolumeMute { muted: bool },
    #[serde(rename = "media.key")]
    MediaKey { key: MediaKey },
    /// Now-playing information of the active media session.
    #[serde(rename = "media.status")]
    MediaStatus,
    #[serde(rename = "process.list")]
    ProcessList,
    /// Graceful stop first (`force=false`); force kill only after the user
    /// confirmed it. `name` must still match `pid` (guards against PID reuse).
    #[serde(rename = "process.terminate")]
    ProcessTerminate { pid: u32, name: String, force: bool },
    /// Top-level application windows (distinct from processes).
    #[serde(rename = "app.list")]
    AppList,
    /// Ask a window to close (like clicking its X); the app may prompt to save.
    #[serde(rename = "app.close")]
    AppClose { window_id: u64, pid: u32 },
    /// The controller removes its own pairing on this computer.
    #[serde(rename = "pairing.unpair")]
    Unpair,
    /// Output devices and per-app audio sessions with capture abilities.
    #[serde(rename = "audio.devices")]
    AudioDevices,
    /// Destination side: open an authenticated UDP sink for `route_id`.
    #[serde(rename = "audio.sink.prepare")]
    SinkPrepare {
        route_id: String,
        /// hex(32 random bytes) chosen by the controller; shared with the source.
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint_id: Option<String>,
        lease_secs: u32,
    },
    #[serde(rename = "audio.sink.stop")]
    SinkStop { route_id: String },
    /// Source side: start streaming to a prepared sink and run the route
    /// state machine up to ACTIVE.
    #[serde(rename = "route.audio.start")]
    RouteStart {
        route_id: String,
        source: RouteSource,
        dest_host: String,
        dest_port: u16,
        key: String,
        lease_secs: u32,
        /// Silence the app locally once remote playback is verified (true handoff).
        silence_local: bool,
    },
    #[serde(rename = "route.audio.stop")]
    RouteStop { route_id: String },
    /// Renew the lease of a route or sink owned by this agent.
    #[serde(rename = "route.lease.renew")]
    LeaseRenew { route_id: String },
    /// All routes and sinks on this agent (with live stats).
    #[serde(rename = "route.list")]
    RouteList,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::DeviceInfo => "device.info",
            Action::TelemetryGet => "telemetry.get",
            Action::VolumeGet => "volume.get",
            Action::VolumeSet { .. } => "volume.set",
            Action::VolumeMute { .. } => "volume.mute",
            Action::MediaKey { .. } => "media.key",
            Action::MediaStatus => "media.status",
            Action::ProcessList => "process.list",
            Action::ProcessTerminate { .. } => "process.terminate",
            Action::AppList => "app.list",
            Action::AppClose { .. } => "app.close",
            Action::Unpair => "pairing.unpair",
            Action::AudioDevices => "audio.devices",
            Action::SinkPrepare { .. } => "audio.sink.prepare",
            Action::SinkStop { .. } => "audio.sink.stop",
            Action::RouteStart { .. } => "route.audio.start",
            Action::RouteStop { .. } => "route.audio.stop",
            Action::LeaseRenew { .. } => "route.lease.renew",
            Action::RouteList => "route.list",
        }
    }

    /// Scope a controller must hold to invoke this action. `None` means any
    /// paired controller may call it (it only affects the caller itself).
    pub fn required_scope(&self) -> Option<Scope> {
        Some(match self {
            Action::DeviceInfo | Action::ProcessList | Action::AppList => Scope::DeviceRead,
            Action::TelemetryGet => Scope::TelemetryRead,
            Action::VolumeGet
            | Action::VolumeSet { .. }
            | Action::VolumeMute { .. }
            | Action::MediaKey { .. }
            | Action::MediaStatus => Scope::MediaControl,
            Action::ProcessTerminate { .. } | Action::AppClose { .. } => Scope::ProcessTerminate,
            Action::AudioDevices
            | Action::SinkPrepare { .. }
            | Action::SinkStop { .. }
            | Action::RouteStart { .. }
            | Action::RouteStop { .. }
            | Action::LeaseRenew { .. }
            | Action::RouteList => Scope::AudioRoute,
            Action::Unpair => return None,
        })
    }

    /// Validate parameters before dispatch.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Action::VolumeSet { level } if !level.is_finite() || !(0.0..=1.0).contains(level) => {
                Err("level must be within 0.0..=1.0".into())
            }
            Action::ProcessTerminate { pid, name, .. }
                if *pid == 0 || name.is_empty() || name.len() > 260 =>
            {
                Err("pid and process name are required".into())
            }
            Action::AppClose { window_id: 0, .. } => Err("window_id is required".into()),
            Action::SinkPrepare {
                route_id,
                key,
                lease_secs,
                ..
            }
            | Action::RouteStart {
                route_id,
                key,
                lease_secs,
                ..
            } if !crate::audio::valid_route_id(route_id)
                || !crate::audio::valid_stream_key(key)
                || !(5..=600).contains(lease_secs) =>
            {
                Err("invalid route_id, key or lease_secs (5..=600)".into())
            }
            Action::RouteStart {
                dest_host,
                dest_port,
                ..
            } if dest_host.is_empty() || dest_host.len() > 253 || *dest_port == 0 => {
                Err("invalid destination".into())
            }
            Action::SinkStop { route_id }
            | Action::RouteStop { route_id }
            | Action::LeaseRenew { route_id }
                if !crate::audio::valid_route_id(route_id) =>
            {
                Err("invalid route_id".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // transient wire value; boxing buys nothing here
pub enum ActionResult {
    Manifest {
        manifest: CapabilityManifest,
    },
    Telemetry(Telemetry),
    Volume(VolumeState),
    MediaStatus(MediaStatus),
    Processes {
        processes: Vec<ProcessInfo>,
    },
    Terminated(TerminateOutcome),
    Apps {
        windows: Vec<AppWindow>,
    },
    Audio(AudioInventory),
    Sink(SinkInfo),
    Route(RouteInfo),
    Routes {
        routes: Vec<RouteInfo>,
        sinks: Vec<SinkInfo>,
    },
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskInfo {
    pub mount: String,
    pub name: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub removable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

/// `active=false` means no media session exists right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaStatus {
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    pub status: PlaybackStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

impl MediaStatus {
    pub fn none() -> Self {
        Self {
            active: false,
            app: None,
            title: None,
            artist: None,
            album: None,
            status: PlaybackStatus::Stopped,
            position_ms: None,
            duration_ms: None,
            can_play_pause: false,
            can_next: false,
            can_previous: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
    pub name: String,
    /// Only present when the OS lets the agent read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    /// Share of total machine CPU (0..=100), like Task Manager.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub status: String,
    pub start_time: u64,
    /// Runs as the same OS user as the agent.
    pub same_user: bool,
    /// Can be ended from ccontrol (same user, not critical, not the agent).
    pub terminable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminateMethod {
    /// Windows: WM_CLOSE to the process's windows. Linux: SIGTERM.
    Graceful,
    Force,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminateOutcome {
    pub pid: u32,
    pub method: TerminateMethod,
    /// Whether the process had exited when the agent stopped waiting.
    pub exited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppWindow {
    pub window_id: u64,
    pub pid: u32,
    pub process_name: String,
    pub title: String,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Telemetry {
    pub cpu_percent: f32,
    pub cpu_count: u32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub uptime_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_charging: Option<bool>,
    #[serde(default)]
    pub disks: Vec<DiskInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeState {
    pub level: f32,
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The controller's trust was revoked; the connection closes next.
    Revoked {
        reason: String,
    },
    VolumeChanged(VolumeState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    IncompatibleProtocol,
    NotPaired,
    IdentityMismatch,
    PairingRejected,
    PairingTimeout,
    PairingFailed,
    ScopeDenied,
    /// Refused by policy (critical process, other user's process, ...).
    Forbidden,
    Unsupported,
    InvalidRequest,
    RateLimited,
    Busy,
    Timeout,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_shape_is_flat() {
        let m = Message::Request {
            id: 3,
            action: Action::VolumeSet { level: 0.25 },
        };
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"type":"request","id":3,"action":"volume.set","level":0.25})
        );
        let back: Message = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn unit_action_round_trip() {
        let m = Message::Request {
            id: 1,
            action: Action::DeviceInfo,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(s, r#"{"type":"request","id":1,"action":"device.info"}"#);
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
    }

    #[test]
    fn response_shapes() {
        let ok = Message::Response {
            id: 9,
            result: Some(ActionResult::Volume(VolumeState {
                level: 0.5,
                muted: false,
                endpoint_name: None,
            })),
            error: None,
        };
        let s = serde_json::to_string(&ok).unwrap();
        assert_eq!(
            s,
            r#"{"type":"response","id":9,"result":{"kind":"volume","level":0.5,"muted":false}}"#
        );
        let err = Message::Response {
            id: 9,
            result: None,
            error: Some(ProtocolError::new(ErrorCode::ScopeDenied, "x")),
        };
        let s = serde_json::to_string(&err).unwrap();
        assert_eq!(
            s,
            r#"{"type":"response","id":9,"error":{"code":"scope_denied","message":"x"}}"#
        );
    }

    #[test]
    fn unknown_action_is_rejected() {
        let r = serde_json::from_str::<Message>(
            r#"{"type":"request","id":1,"action":"shell.exec","cmd":"rm -rf /"}"#,
        );
        assert!(r.is_err());
    }

    #[test]
    fn phase2_action_shapes() {
        let m = Message::Request {
            id: 4,
            action: Action::ProcessTerminate {
                pid: 42,
                name: "notepad.exe".into(),
                force: false,
            },
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"type":"request","id":4,"action":"process.terminate","pid":42,"name":"notepad.exe","force":false}"#
        );
        let m = Message::Request {
            id: 5,
            action: Action::AppClose {
                window_id: 7,
                pid: 42,
            },
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"type":"request","id":5,"action":"app.close","window_id":7,"pid":42}"#
        );
        assert_eq!(Action::Unpair.required_scope(), None);
        assert_eq!(
            Action::AppClose {
                window_id: 1,
                pid: 1
            }
            .required_scope(),
            Some(Scope::ProcessTerminate)
        );
        let r = ActionResult::Terminated(TerminateOutcome {
            pid: 42,
            method: TerminateMethod::Graceful,
            exited: true,
            note: None,
        });
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"kind":"terminated","pid":42,"method":"graceful","exited":true}"#
        );
        // Telemetry from Phase 1 agents (no disks) still decodes.
        let t: Telemetry = serde_json::from_str(
            r#"{"cpu_percent":1.0,"cpu_count":2,"memory_used_bytes":1,"memory_total_bytes":2,"uptime_secs":3}"#,
        )
        .unwrap();
        assert!(t.disks.is_empty());
    }

    #[test]
    fn validates_volume_range() {
        assert!(Action::VolumeSet { level: 1.5 }.validate().is_err());
        assert!(Action::VolumeSet { level: f32::NAN }.validate().is_err());
        assert!(Action::VolumeSet { level: 0.0 }.validate().is_ok());
    }

    #[test]
    fn event_shape() {
        let m = Message::Event {
            event: AgentEvent::Revoked {
                reason: "owner".into(),
            },
        };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(s, r#"{"type":"event","event":"revoked","reason":"owner"}"#);
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
    }
}
