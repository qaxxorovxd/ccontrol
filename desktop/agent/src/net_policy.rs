use std::net::IpAddr;

/// LAN / VPN / loopback source addresses accepted by default.
pub fn is_private_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                // 100.64.0.0/10 carrier-grade NAT range, used by Tailscale.
                || (o[0] == 100 && (o[1] & 0xC0) == 64)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_peer(IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link local fe80::/10
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        for ok in [
            "192.168.1.5",
            "10.0.0.1",
            "172.20.1.1",
            "127.0.0.1",
            "100.101.1.1",
            "fe80::1",
            "fd00::5",
            "::ffff:192.168.0.2",
        ] {
            assert!(is_private_peer(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "2001:4860::8888",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_private_peer(bad.parse().unwrap()), "{bad}");
        }
    }
}
