use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

/// Persistent agent configuration (`config.json` in the data directory).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Name shown to controllers; defaults to the hostname.
    pub device_name: Option<String>,
    /// Address the authenticated control listener binds to.
    pub listen_addr: IpAddr,
    pub listen_port: u16,
    /// Loopback-only settings UI port.
    pub admin_port: u16,
    /// Accept control connections from non-private (public) source addresses.
    /// Off by default: ccontrol is LAN/VPN only.
    pub allow_public_peers: bool,
    pub mdns_enabled: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            device_name: None,
            listen_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            listen_port: ccontrol_protocol::DEFAULT_PORT,
            admin_port: ccontrol_protocol::DEFAULT_PORT + 1,
            allow_public_peers: false,
            mdns_enabled: true,
        }
    }
}

impl AgentConfig {
    pub fn load_or_default(dir: &Path) -> anyhow::Result<Self> {
        let p = dir.join("config.json");
        if p.exists() {
            Ok(serde_json::from_slice(&std::fs::read(&p)?)?)
        } else {
            let c = Self::default();
            std::fs::create_dir_all(dir)?;
            std::fs::write(&p, serde_json::to_vec_pretty(&c)?)?;
            Ok(c)
        }
    }
}

/// Per-user data directory (`%APPDATA%\ccontrol\agent`, `~/.local/share/ccontrol/agent`).
pub fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "ccontrol", "ccontrol")
        .map(|d| d.data_dir().join("agent"))
        .unwrap_or_else(|| PathBuf::from(".ccontrol-agent"))
}
