//! DNS-SD advertisement (`_ccontrol._tcp`). Discovery is only a hint: the
//! TXT record is unauthenticated, and trust is always established by the
//! certificate fingerprint during the TLS handshake.

use ccontrol_protocol::{MDNS_SERVICE_TYPE, PROTOCOL_VERSION};
use mdns_sd::{ServiceDaemon, ServiceInfo};

#[derive(Debug, thiserror::Error)]
#[error("mdns: {0}")]
pub struct DiscoveryError(String);

pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

pub struct AdvertiseParams<'a> {
    pub device_id: &'a str,
    pub device_name: &'a str,
    pub os_family: &'a str,
    pub port: u16,
}

/// Sanitized DNS-SD instance label (max 63 bytes, no dots).
pub fn instance_name(device_name: &str, device_id: &str) -> String {
    let mut base: String = device_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    base.truncate(40);
    format!("{} {}", base.trim(), &device_id[..device_id.len().min(8)])
}

impl Advertiser {
    pub fn start(p: AdvertiseParams<'_>) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(|e| DiscoveryError(e.to_string()))?;
        let host = format!(
            "ccontrol-{}.local.",
            &p.device_id[..p.device_id.len().min(12)]
        );
        let props = [
            ("v", PROTOCOL_VERSION.to_string()),
            ("id", p.device_id.to_owned()),
            ("name", p.device_name.chars().take(60).collect::<String>()),
            ("os", p.os_family.to_owned()),
        ];
        let info = ServiceInfo::new(
            MDNS_SERVICE_TYPE,
            &instance_name(p.device_name, p.device_id),
            &host,
            "",
            p.port,
            &props[..],
        )
        .map_err(|e| DiscoveryError(e.to_string()))?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        daemon
            .register(info)
            .map_err(|e| DiscoveryError(e.to_string()))?;
        tracing::info!(service = %fullname, "mDNS advertisement registered");
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn instance_names_are_safe() {
        let n = super::instance_name("Kai's.PC", "abcdef0123456789");
        assert!(!n.contains('.'));
        assert!(n.ends_with("abcdef01"));
    }
}
