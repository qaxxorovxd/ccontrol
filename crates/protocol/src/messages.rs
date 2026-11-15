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
        }
    }

    /// Scope a controller must hold to invoke this action.
    pub fn required_scope(&self) -> Scope {
        match self {
            Action::DeviceInfo => Scope::DeviceRead,
            Action::TelemetryGet => Scope::TelemetryRead,
            Action::VolumeGet | Action::VolumeSet { .. } | Action::VolumeMute { .. } => {
                Scope::MediaControl
            }
            Action::MediaKey { .. } => Scope::MediaControl,
        }
    }

    /// Validate parameters before dispatch.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Action::VolumeSet { level } if !level.is_finite() || !(0.0..=1.0).contains(level) => {
                Err("level must be within 0.0..=1.0".into())
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
    Manifest { manifest: CapabilityManifest },
    Telemetry(Telemetry),
    Volume(VolumeState),
    Done,
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
