//! Linux adapters.
//!
//! * Volume: PipeWire's `wpctl` (WirePlumber), falling back to PulseAudio's
//!   `pactl`. Executables are fixed and arguments typed — no shell is used.
//!   Native libpipewire bindings replace this in the audio-engine milestone.
//! * Media: MPRIS over the D-Bus session bus.

use crate::{check_level, ControlError, MediaControl, VolumeControl};
use ccontrol_protocol::{MediaKey, MediaStatus, PlaybackStatus, VolumeState};
use std::collections::HashMap;
use std::process::Command;
use zbus::zvariant::OwnedValue;

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

const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";

fn session() -> Result<zbus::blocking::Connection, ControlError> {
    zbus::blocking::Connection::session()
        .map_err(|e| ControlError::Unsupported(format!("no D-Bus session bus: {e}")))
}

fn player_proxy<'a>(
    conn: &'a zbus::blocking::Connection,
    name: &'a str,
) -> Result<zbus::blocking::Proxy<'a>, ControlError> {
    zbus::blocking::Proxy::new(conn, name, MPRIS_PATH, PLAYER_IFACE)
        .map_err(|e| ControlError::Platform(e.to_string()))
}

/// Pick the player that is playing, else the first one found.
fn pick_player(conn: &zbus::blocking::Connection) -> Result<Option<String>, ControlError> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn)
        .map_err(|e| ControlError::Platform(e.to_string()))?;
    let names: Vec<String> = dbus
        .list_names()
        .map_err(|e| ControlError::Platform(e.to_string()))?
        .iter()
        .map(|n| n.as_str().to_owned())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .collect();
    for n in &names {
        if let Ok(p) = player_proxy(conn, n) {
            if p.get_property::<String>("PlaybackStatus").ok().as_deref() == Some("Playing") {
                return Ok(Some(n.clone()));
            }
        }
    }
    Ok(names.into_iter().next())
}

fn meta_str(m: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    let v = m.get(k)?.try_clone().ok()?;
    String::try_from(v).ok().filter(|s| !s.is_empty())
}

fn meta_artists(m: &HashMap<String, OwnedValue>) -> Option<String> {
    let v = m.get("xesam:artist")?.try_clone().ok()?;
    Vec::<String>::try_from(v)
        .ok()
        .map(|a| a.join(", "))
        .filter(|s| !s.is_empty())
}

impl MediaControl for Mpris {
    fn send(&self, key: MediaKey) -> Result<(), ControlError> {
        let conn = session()?;
        let player = pick_player(&conn)?
            .ok_or_else(|| ControlError::Unsupported("no MPRIS media player is running".into()))?;
        let method = match key {
            MediaKey::PlayPause => "PlayPause",
            MediaKey::Next => "Next",
            MediaKey::Previous => "Previous",
            MediaKey::Stop => "Stop",
        };
        conn.call_method(
            Some(player.as_str()),
            MPRIS_PATH,
            Some(PLAYER_IFACE),
            method,
            &(),
        )
        .map_err(|e| ControlError::Platform(e.to_string()))?;
        Ok(())
    }

    fn status(&self) -> Result<MediaStatus, ControlError> {
        let conn = session()?;
        let Some(name) = pick_player(&conn)? else {
            return Ok(MediaStatus::none());
        };
        let p = player_proxy(&conn, &name)?;
        let status = match p.get_property::<String>("PlaybackStatus").ok().as_deref() {
            Some("Playing") => PlaybackStatus::Playing,
            Some("Paused") => PlaybackStatus::Paused,
            Some("Stopped") => PlaybackStatus::Stopped,
            _ => PlaybackStatus::Unknown,
        };
        let meta: HashMap<String, OwnedValue> = p.get_property("Metadata").unwrap_or_default();
        // mpris:length and Position are microseconds.
        let duration_ms = meta
            .get("mpris:length")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| i64::try_from(v).ok().or(None))
            .filter(|d| *d > 0)
            .map(|d| (d / 1000) as u64);
        let position_ms = p
            .get_property::<i64>("Position")
            .ok()
            .filter(|_| duration_ms.is_some())
            .map(|d| (d.max(0) / 1000) as u64);
        let flag = |k: &str| p.get_property::<bool>(k).unwrap_or(false);
        Ok(MediaStatus {
            active: true,
            app: Some(
                name.trim_start_matches("org.mpris.MediaPlayer2.")
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            ),
            title: meta_str(&meta, "xesam:title"),
            artist: meta_artists(&meta),
            album: meta_str(&meta, "xesam:album"),
            status,
            position_ms,
            duration_ms,
            can_play_pause: flag("CanPlay") || flag("CanPause"),
            can_next: flag("CanGoNext"),
            can_previous: flag("CanGoPrevious"),
        })
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
