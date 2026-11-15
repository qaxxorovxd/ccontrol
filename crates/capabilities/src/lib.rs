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
    pub media: Option<Result<(), String>>,
    pub windows: Option<Result<(), String>>,
    pub audio: Option<Result<AudioProbe, String>>,
}

#[derive(Debug, Clone, Default)]
pub struct AudioProbe {
    pub can_play: bool,
    pub endpoint_capture: bool,
    pub process_capture: bool,
    pub process_capture_reason: Option<String>,
}

pub fn probe() -> Probes {
    let vol = ccontrol_system_control::volume()
        .get()
        .map(|_| ())
        .map_err(|e| e.to_string());
    let media = ccontrol_system_control::media()
        .status()
        .map(|_| ())
        .map_err(|e| e.to_string());
    let windows = ccontrol_system_control::windows_adapter()
        .list()
        .map(|_| ())
        .map_err(|e| e.to_string());
    Probes {
        volume: Some(vol),
        media: Some(media),
        windows: Some(windows),
        audio: None,
    }
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

    let from_probe = |p: &Option<Result<(), String>>, scope: Scope, note: Option<&str>| match p {
        Some(Ok(())) => CapabilityStatus {
            available: true,
            reason: note.map(str::to_owned),
            scope: Some(scope.as_str().into()),
        },
        Some(Err(e)) => CapabilityStatus::unavailable(e.clone(), Some(scope.as_str())),
        None => CapabilityStatus::unavailable("not probed", Some(scope.as_str())),
    };
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
    put(
        "media.status",
        from_probe(
            &probes.media,
            Scope::MediaControl,
            Some(if os.family == "windows" {
                "System Media Transport Controls (apps that publish now-playing info)"
            } else {
                "MPRIS players on the D-Bus session bus"
            }),
        ),
    );

    let route_scope = Some(Scope::AudioRoute.as_str());
    match &probes.audio {
        Some(Ok(a)) => {
            put(
                "audio.per_app_capture",
                if a.process_capture {
                    CapabilityStatus {
                        available: true,
                        reason: Some(if os.family == "linux" {
                            "PipeWire stream capture (local playback is kept on Linux)".into()
                        } else {
                            "Windows process loopback".into()
                        }),
                        scope: route_scope.map(str::to_owned),
                    }
                } else {
                    CapabilityStatus::unavailable(
                        a.process_capture_reason
                            .clone()
                            .unwrap_or_else(|| "Not supported".into()),
                        route_scope,
                    )
                },
            );
            put(
                "audio.capture_output",
                if a.endpoint_capture {
                    CapabilityStatus::available(route_scope)
                } else {
                    CapabilityStatus::unavailable("No output device to capture", route_scope)
                },
            );
            put(
                "audio.playback",
                if a.can_play {
                    CapabilityStatus::available(route_scope)
                } else {
                    CapabilityStatus::unavailable("No output device", route_scope)
                },
            );
            put(
                "audio.route",
                CapabilityStatus {
                    available: a.can_play || a.endpoint_capture,
                    reason: Some("Opus over authenticated UDP; source agent owns the route and restores it on stop/crash".into()),
                    scope: route_scope.map(str::to_owned),
                },
            );
        }
        Some(Err(e)) => {
            for k in [
                "audio.per_app_capture",
                "audio.capture_output",
                "audio.playback",
                "audio.route",
            ] {
                put(k, CapabilityStatus::unavailable(e.clone(), route_scope));
            }
        }
        None => {
            for k in [
                "audio.per_app_capture",
                "audio.capture_output",
                "audio.playback",
                "audio.route",
            ] {
                put(k, not_yet(Scope::AudioRoute));
            }
        }
    }

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
    put(
        "process.list",
        CapabilityStatus::available(Some(Scope::DeviceRead.as_str())),
    );
    put(
        "process.terminate",
        CapabilityStatus {
            available: true,
            reason: Some("Own-user processes only; critical system processes are protected".into()),
            scope: Some(Scope::ProcessTerminate.as_str().into()),
        },
    );
    put(
        "apps.windows",
        from_probe(&probes.windows, Scope::DeviceRead, None),
    );
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
