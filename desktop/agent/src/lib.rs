//! ccontrol desktop agent library (used by the binary and integration tests).

pub mod admin;
pub mod audio;
pub mod autostart;
pub mod config;
pub mod dispatch;
pub mod net_policy;
pub mod pairing;
pub mod server;
pub mod sessions;
pub mod state;

pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");
