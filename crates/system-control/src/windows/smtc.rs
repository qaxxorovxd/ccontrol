//! System Media Transport Controls (Windows 10 1809+): the same now-playing
//! session the volume flyout shows. Commands target that session; if no
//! session exists, media keys are sent as a fallback.

use super::{plat, send_media_key, Com};
use crate::{ControlError, MediaControl};
use ccontrol_protocol::{MediaKey, MediaStatus, PlaybackStatus};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

pub struct WindowsMedia;

fn current_session() -> Result<Option<Session>, ControlError> {
    let mgr = Manager::RequestAsync().map_err(plat)?.get().map_err(|e| {
        ControlError::Unsupported(format!("media session manager unavailable: {e}"))
    })?;
    // GetCurrentSession fails (null) when nothing is playing.
    Ok(mgr.GetCurrentSession().ok())
}

/// TimeSpan is in 100 ns ticks.
fn ticks_to_ms(t: windows::Foundation::TimeSpan) -> u64 {
    (t.Duration.max(0) / 10_000) as u64
}

fn non_empty(h: windows::core::Result<windows::core::HSTRING>) -> Option<String> {
    h.ok()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

/// "Spotify.exe" / "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" -> readable name.
fn app_label(aumid: &str) -> String {
    let base = aumid.rsplit('!').next().unwrap_or(aumid);
    let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
    base.rsplit('.').next().unwrap_or(base).to_owned()
}

impl MediaControl for WindowsMedia {
    fn send(&self, key: MediaKey) -> Result<(), ControlError> {
        let _com = Com::init();
        if let Ok(Some(s)) = current_session() {
            let op = match key {
                MediaKey::PlayPause => s.TryTogglePlayPauseAsync(),
                MediaKey::Next => s.TrySkipNextAsync(),
                MediaKey::Previous => s.TrySkipPreviousAsync(),
                MediaKey::Stop => s.TryStopAsync(),
            };
            if let Ok(true) = op.and_then(|o| o.get()) {
                return Ok(());
            }
        }
        send_media_key(key)
    }

    fn status(&self) -> Result<MediaStatus, ControlError> {
        let _com = Com::init();
        let Some(s) = current_session()? else {
            return Ok(MediaStatus::none());
        };
        let props = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()).ok();
        let info = s.GetPlaybackInfo().map_err(plat)?;
        let status = match info.PlaybackStatus().map_err(plat)? {
            Status::Playing => PlaybackStatus::Playing,
            Status::Paused => PlaybackStatus::Paused,
            Status::Stopped | Status::Closed => PlaybackStatus::Stopped,
            _ => PlaybackStatus::Unknown,
        };
        let controls = info.Controls().ok();
        let flag = |f: fn(
            &windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackControls,
        ) -> windows::core::Result<bool>| {
            controls.as_ref().and_then(|c| f(c).ok()).unwrap_or(false)
        };
        let timeline = s.GetTimelineProperties().ok();
        let duration_ms = timeline
            .as_ref()
            .and_then(|t| {
                Some(
                    ticks_to_ms(t.EndTime().ok()?).saturating_sub(ticks_to_ms(t.StartTime().ok()?)),
                )
            })
            .filter(|d| *d > 0);
        let position_ms = timeline
            .as_ref()
            .and_then(|t| t.Position().ok())
            .map(ticks_to_ms)
            .filter(|_| duration_ms.is_some());
        Ok(MediaStatus {
            active: true,
            app: non_empty(s.SourceAppUserModelId()).map(|a| app_label(&a)),
            title: props.as_ref().and_then(|p| non_empty(p.Title())),
            artist: props.as_ref().and_then(|p| non_empty(p.Artist())),
            album: props.as_ref().and_then(|p| non_empty(p.AlbumTitle())),
            status,
            position_ms,
            duration_ms,
            can_play_pause: flag(|c| c.IsPlayPauseToggleEnabled())
                || flag(|c| c.IsPlayEnabled())
                || flag(|c| c.IsPauseEnabled()),
            can_next: flag(|c| c.IsNextEnabled()),
            can_previous: flag(|c| c.IsPreviousEnabled()),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels() {
        assert_eq!(super::app_label("Spotify.exe"), "Spotify");
        assert_eq!(
            super::app_label("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(super::app_label("308046B0AF4A39CB"), "308046B0AF4A39CB");
    }

    /// Talks to the real SMTC manager; must not fail even when nothing plays.
    #[test]
    fn status_does_not_error() {
        use crate::MediaControl;
        // CI runners (Windows Server) may have no media session manager: that
        // must surface as "unsupported", never as a crash or platform error.
        match super::WindowsMedia.status() {
            Ok(_) | Err(crate::ControlError::Unsupported(_)) => {}
            Err(e) => panic!("{e:?}"),
        }
    }
}
