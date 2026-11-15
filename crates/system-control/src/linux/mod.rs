//! Linux adapters.
//!
//! * Volume: PipeWire's `wpctl` (WirePlumber), falling back to PulseAudio's
//!   `pactl`. Executables are fixed and arguments typed — no shell is used.
//!   Native libpipewire bindings replace this in the audio-engine milestone.
//! * Media: MPRIS over the D-Bus session bus.

use crate::{check_level, ControlError, MediaControl, VolumeControl};
use ccontrol_protocol::{MediaKey, VolumeState};
use std::process::Command;

fn run(exe: &str, args: &[&str]) -> Result<String, ControlError> {
    let out = Command::new(exe)
        .args(args)
        .output()
        .map_err(|e| ControlError::Unsupported(format!("{exe} not available: {e}")))?;
    if !out.status.success() {
        return Err(ControlError::Platform(format!(
            "{exe} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub struct CliVolume;

/// Parse `wpctl get-volume` output: `Volume: 0.40` or `Volume: 0.40 [MUTED]`.
pub(crate) fn parse_wpctl(s: &str) -> Option<VolumeState> {
    let rest = s.trim().strip_prefix("Volume:")?.trim();
    let mut parts = rest.split_whitespace();
    let level: f32 = parts.next()?.parse().ok()?;
    let muted = rest.contains("[MUTED]");
    Some(VolumeState {
        level: level.clamp(0.0, 1.5),
        muted,
        endpoint_name: None,
    })
}

/// Parse the first percentage of `pactl get-sink-volume @DEFAULT_SINK@`.
pub(crate) fn parse_pactl_volume(s: &str) -> Option<f32> {
    let pct = s.split_whitespace().find(|w| w.ends_with('%'))?;
    pct.trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .map(|p| p / 100.0)
}

impl CliVolume {
    fn wpctl_get() -> Result<VolumeState, ControlError> {
        let out = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
        parse_wpctl(&out)
            .ok_or_else(|| ControlError::Platform(format!("unexpected wpctl output: {out}")))
    }

    fn pactl_get() -> Result<VolumeState, ControlError> {
        let v = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
        let m = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])?;
        let level = parse_pactl_volume(&v)
            .ok_or_else(|| ControlError::Platform(format!("unexpected pactl output: {v}")))?;
        Ok(VolumeState {
            level,
            muted: m.contains("yes"),
            endpoint_name: None,
        })
    }

    fn has_wpctl() -> bool {
        Command::new("wpctl").arg("--help").output().is_ok()
    }
}

impl VolumeControl for CliVolume {
    fn get(&self) -> Result<VolumeState, ControlError> {
        Self::wpctl_get().or_else(|_| Self::pactl_get())
    }

    fn set_level(&self, level: f32) -> Result<VolumeState, ControlError> {
        let level = check_level(level)?;
        if Self::has_wpctl() {
            run(
                "wpctl",
                &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{level:.3}")],
            )?;
        } else {
            let pct = format!("{}%", (level * 100.0).round() as u32);
            run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &pct])?;
        }
        self.get()
    }

    fn set_muted(&self, muted: bool) -> Result<VolumeState, ControlError> {
        let flag = if muted { "1" } else { "0" };
        if Self::has_wpctl() {
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", flag])?;
        } else {
            run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", flag])?;
        }
        self.get()
    }
}

pub struct Mpris;

impl MediaControl for Mpris {
    fn send(&self, key: MediaKey) -> Result<(), ControlError> {
        let conn = zbus::blocking::Connection::session()
            .map_err(|e| ControlError::Unsupported(format!("no D-Bus session bus: {e}")))?;
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)
            .map_err(|e| ControlError::Platform(e.to_string()))?;
        let names = dbus
            .list_names()
            .map_err(|e| ControlError::Platform(e.to_string()))?;
        let player = names
            .iter()
            .map(|n| n.as_str().to_owned())
            .find(|n| n.starts_with("org.mpris.MediaPlayer2."))
            .ok_or_else(|| ControlError::Unsupported("no MPRIS media player is running".into()))?;
        let method = match key {
            MediaKey::PlayPause => "PlayPause",
            MediaKey::Next => "Next",
            MediaKey::Previous => "Previous",
            MediaKey::Stop => "Stop",
        };
        conn.call_method(
            Some(player.as_str()),
            "/org/mpris/MediaPlayer2",
            Some("org.mpris.MediaPlayer2.Player"),
            method,
            &(),
        )
        .map_err(|e| ControlError::Platform(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl() {
        assert_eq!(parse_wpctl("Volume: 0.40\n").unwrap().level, 0.40);
        let m = parse_wpctl("Volume: 0.15 [MUTED]").unwrap();
        assert!(m.muted);
        assert!(parse_wpctl("garbage").is_none());
    }

    #[test]
    fn parses_pactl() {
        let s =
            "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB";
        assert_eq!(parse_pactl_volume(s), Some(0.40));
    }
}
