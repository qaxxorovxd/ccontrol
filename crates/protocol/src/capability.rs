use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Availability of one capability on one device, as detected by the native agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityStatus {
    pub available: bool,
    /// Human-readable reason when unavailable (or caveat when available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Scope a controller needs to use the capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl CapabilityStatus {
    pub fn available(scope: Option<&str>) -> Self {
        Self {
            available: true,
            reason: None,
            scope: scope.map(str::to_owned),
        }
    }
    pub fn unavailable(reason: impl Into<String>, scope: Option<&str>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            scope: scope.map(str::to_owned),
        }
    }
}

/// Trusted capability manifest sent by an agent after authentication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub device_id: String,
    pub device_name: String,
    pub os_family: String,
    pub os_distribution: Option<String>,
    pub os_version: String,
    pub os_build: Option<String>,
    pub kernel_version: Option<String>,
    pub architecture: String,
    /// windows | x11 | wayland | tty | headless | unknown
    pub session_type: String,
    pub agent_version: String,
    pub protocol_version: u32,
    pub capabilities: BTreeMap<String, CapabilityStatus>,
}
