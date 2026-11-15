//! Builds the trusted capability manifest from *native* detection.
//!
//! Every capability is reported with the reason it is unavailable. Nothing is
//! inferred from hostnames or from the controller.

use ccontrol_protocol::{CapabilityManifest, CapabilityStatus, Scope, PROTOCOL_VERSION};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct OsInfo {
    pub family: String,
    pub distribution: Option<String>,
    pub version: String,
    pub build: Option<String>,
    pub kernel: Option<String>,
    pub arch: String,
    pub session_type: String,
    pub hostname: String,
}

pub fn detect_os() -> OsInfo {
    let family = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    };
    let distribution = if cfg!(target_os = "linux") {
        sysinfo::System::distribution_id().into()
    } else {
        None
    };
    let version = sysinfo::System::long_os_version()
        .or_else(sysinfo::System::os_version)
        .unwrap_or_else(|| "unknown".into());
    OsInfo {
        family: family.into(),
        distribution: distribution.filter(|d: &String| !d.is_empty()),
        version,
        build: windows_build().map(|b| b.to_string()),
        kernel: sysinfo::System::kernel_version(),
        arch: std::env::consts::ARCH.into(),
        session_type: session_type(),
        hostname: sysinfo::System::host_name().unwrap_or_else(|| "computer".into()),
    }
}

/// Windows build number (e.g. 19045, 22631) from `sysinfo` kernel version.
pub fn windows_build() -> Option<u32> {
    if !cfg!(windows) {
        return None;
    }
    // sysinfo returns e.g. "19045" or "10.0.19045" depending on version.
    let k = sysinfo::System::kernel_version()?;
    k.rsplit('.').next()?.trim().parse().ok()
}

fn session_type() -> String {
    if cfg!(windows) {
        return "windows".into();
    }
    match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") => "wayland".into(),
        Some("x11") => "x11".into(),
        Some("tty") => "tty".into(),
        _ if std::env::var_os("WAYLAND_DISPLAY").is_some() => "wayland".into(),
        _ if std::env::var_os("DISPLAY").is_some() => "x11".into(),
        _ => "headless".into(),
    }
}

/// Results of live probes performed by the agent at startup / on request.
#[derive(Debug, Clone, Default)]
pub struct Probes {
    pub volume: Option<Result<(), String>>,
}

pub fn probe() -> Probes {
    let vol = ccontrol_system_control::volume()
        .get()
        .map(|_| ())
        .map_err(|e| e.to_string());
    Probes { volume: Some(vol) }
}

pub fn build_manifest(
    device_id: &str,
    device_name: &str,
    agent_version: &str,
    os: &OsInfo,
    probes: &Probes,
) -> CapabilityManifest {
    let mut caps = BTreeMap::new();
    let mut put = |k: &str, v: CapabilityStatus| {
        caps.insert(k.to_owned(), v);
    };
    let not_yet = |scope: Scope| {
        CapabilityStatus::unavailable("Not implemented in this agent build", Some(scope.as_str()))
    };

    put(
        "device.info",
        CapabilityStatus::available(Some(Scope::DeviceRead.as_str())),
    );
    put(
        "telemetry.basic",
        CapabilityStatus::available(Some(Scope::TelemetryRead.as_str())),
    );

    match &probes.volume {
        Some(Ok(())) => put(
            "audio.master_volume",
            CapabilityStatus::available(Some(Scope::MediaControl.as_str())),
        ),
        Some(Err(e)) => put(
            "audio.master_volume",
            CapabilityStatus::unavailable(e.clone(), Some(Scope::MediaControl.as_str())),
        ),
        None => put(
            "audio.master_volume",
            CapabilityStatus::unavailable("not probed", Some(Scope::MediaControl.as_str())),
        ),
    }

    let media = match os.family.as_str() {
        "windows" => CapabilityStatus {
            available: true,
            reason: Some("Sends global media keys; requires the agent to run in the interactive user session".into()),
            scope: Some(Scope::MediaControl.as_str().into()),
        },
        "linux" if os.session_type != "headless" => CapabilityStatus {
            available: true,
            reason: Some("MPRIS over the D-Bus session bus; needs a running MPRIS player".into()),
            scope: Some(Scope::MediaControl.as_str().into()),
        },
        _ => CapabilityStatus::unavailable("No user session", Some(Scope::MediaControl.as_str())),
    };
    put("media.keys", media);

    // Per-app audio capture: report OS-level feasibility truthfully, but the
    // engine is not shipped yet, so it is unavailable.
    let per_app = match (os.family.as_str(), windows_build()) {
        ("windows", Some(b)) if b < 20348 => format!(
            "Windows build {b} lacks process-loopback capture (needs build 20348+); engine not implemented yet"
        ),
        ("windows", _) => "OS supports process-loopback capture; audio engine not implemented yet".into(),
        ("linux", _) => "PipeWire per-stream capture possible; audio engine not implemented yet".into(),
        _ => "Unsupported OS".into(),
    };
    put(
        "audio.per_app_capture",
        CapabilityStatus::unavailable(per_app, Some(Scope::AudioRoute.as_str())),
    );
    put("audio.route", not_yet(Scope::AudioRoute));

    let input_reason = match os.session_type.as_str() {
        "wayland" => {
            "Wayland requires the RemoteDesktop portal with user consent; not implemented yet"
        }
        "headless" | "tty" => "No graphical session",
        _ => "Not implemented in this agent build",
    };
    put(
        "input.keyboard",
        CapabilityStatus::unavailable(input_reason, Some(Scope::InputControl.as_str())),
    );
    put(
        "input.mouse",
        CapabilityStatus::unavailable(input_reason, Some(Scope::InputControl.as_str())),
    );
    put(
        "screen.capture",
        CapabilityStatus::unavailable(
            if os.session_type == "wayland" {
                "Requires ScreenCast portal consent; not implemented yet"
            } else {
                "Not implemented in this agent build"
            },
            Some(Scope::InputControl.as_str()),
        ),
    );
    put("notifications.mirror", not_yet(Scope::NotificationsRead));
    put("process.list", not_yet(Scope::DeviceRead));
    put("files.browse", not_yet(Scope::FilesRead));
    put("power.control", not_yet(Scope::PowerControl));
    put("commands.custom", not_yet(Scope::CommandsRun));

    CapabilityManifest {
        device_id: device_id.into(),
        device_name: device_name.into(),
        os_family: os.family.clone(),
        os_distribution: os.distribution.clone(),
        os_version: os.version.clone(),
        os_build: os.build.clone(),
        kernel_version: os.kernel.clone(),
        architecture: os.arch.clone(),
        session_type: os.session_type.clone(),
        agent_version: agent_version.into(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: caps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_this_host() {
        let os = detect_os();
        assert!(!os.version.is_empty());
        assert_eq!(os.arch, std::env::consts::ARCH);
        if cfg!(windows) {
            assert_eq!(os.family, "windows");
            assert!(windows_build().is_some());
        }
    }

    #[test]
    fn unprobed_features_are_unavailable_with_reason() {
        let os = detect_os();
        let m = build_manifest("id", "pc", "0.1.0", &os, &Probes::default());
        for (k, v) in &m.capabilities {
            if !v.available {
                assert!(v.reason.is_some(), "{k} unavailable without reason");
            }
        }
        assert!(!m.capabilities["audio.route"].available);
        assert!(!m.capabilities["audio.master_volume"].available);
    }
}
