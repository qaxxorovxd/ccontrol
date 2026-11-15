//! ccontrol control protocol.
//!
//! Wire format: every frame is a 4-byte big-endian length followed by a UTF-8
//! JSON document encoding a [`Message`]. Frames are carried inside a mutually
//! authenticated TLS 1.3 channel (see `ccontrol-security`). JSON was chosen
//! over Protobuf for Phase 1 (see `docs/protocols/CONTROL_PROTOCOL.md`); the
//! message set is closed and versioned so a later switch only affects framing.

pub mod capability;
pub mod frame;
pub mod messages;
pub mod scope;

pub use capability::{CapabilityManifest, CapabilityStatus};
pub use frame::{decode_payload, encode_frame, FrameError, MAX_FRAME_LEN};
#[cfg(feature = "tokio")]
pub use frame::{read_frame, write_frame};
pub use messages::*;
pub use scope::Scope;

/// Current protocol version spoken by this build.
pub const PROTOCOL_VERSION: u32 = 1;
/// Oldest protocol version this build can still talk to.
pub const MIN_PROTOCOL_VERSION: u32 = 1;
/// Default TCP port of the desktop agent's control listener.
pub const DEFAULT_PORT: u16 = 47800;
/// DNS-SD service type advertised by desktop agents.
pub const MDNS_SERVICE_TYPE: &str = "_ccontrol._tcp.local.";

/// Returns true when a peer advertising `peer_version` can be served.
pub fn is_compatible(peer_version: u32) -> bool {
    (MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&peer_version)
}
