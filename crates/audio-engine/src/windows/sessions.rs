//! Endpoint and audio-session enumeration, session mute.

use super::win;
use crate::AudioError;
use ccontrol_protocol::{AudioEndpoint, AudioSession};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use windows::core::{Interface, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
    IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

pub(crate) fn enumerator() -> Result<IMMDeviceEnumerator, AudioError> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(win)
}

unsafe fn take_pwstr(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

pub(crate) fn device_id(d: &IMMDevice) -> Option<String> {
    unsafe { d.GetId().ok().and_then(|p| take_pwstr(p)) }
}

fn friendly_name(d: &IMMDevice) -> Option<String> {
    unsafe {
        let store = d.OpenPropertyStore(STGM_READ).ok()?;
        let pv = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        take_pwstr(PropVariantToStringAlloc(&pv).ok()?)
    }
}

/// The default render endpoint, or the one with `id`.
pub(crate) fn device(id: Option<&str>) -> Result<IMMDevice, AudioError> {
    let en = enumerator()?;
    unsafe {
        match id {
            Some(id) => {
                let w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
                en.GetDevice(windows::core::PCWSTR(w.as_ptr()))
                    .map_err(|e| AudioError::Invalid(format!("output device not found: {e}")))
            }
            None => en
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| AudioError::Unsupported(format!("no default output device: {e}"))),
        }
    }
}

pub(crate) fn endpoint_name(id: Option<&str>) -> Result<String, AudioError> {
    let d = device(id)?;
    Ok(friendly_name(&d).unwrap_or_else(|| "Output".into()))
}

pub(crate) fn endpoints() -> Result<Vec<AudioEndpoint>, AudioError> {
    let en = enumerator()?;
    let default_id = unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole) }
        .ok()
        .and_then(|d| device_id(&d));
    let coll = unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }.map_err(win)?;
    let n = unsafe { coll.GetCount() }.map_err(win)?;
    let mut out = vec![];
    for i in 0..n {
        let Ok(d) = (unsafe { coll.Item(i) }) else {
            continue;
        };
        let Some(id) = device_id(&d) else { continue };
        out.push(AudioEndpoint {
            is_default: Some(&id) == default_id.as_ref(),
            name: friendly_name(&d).unwrap_or_else(|| "Output".into()),
            id,
        });
    }
    Ok(out)
}

struct RawSession {
    ctl: IAudioSessionControl2,
    instance: String,
    pid: u32,
}

fn raw_sessions() -> Result<Vec<RawSession>, AudioError> {
    let dev = device(None)?;
    let mgr: IAudioSessionManager2 = unsafe { dev.Activate(CLSCTX_ALL, None) }.map_err(win)?;
    let en = unsafe { mgr.GetSessionEnumerator() }.map_err(win)?;
    let n = unsafe { en.GetCount() }.map_err(win)?;
    let mut out = vec![];
    for i in 0..n {
        let Ok(c) = (unsafe { en.GetSession(i) }) else {
            continue;
        };
        let Ok(ctl) = c.cast::<IAudioSessionControl2>() else {
            continue;
        };
        let Some(instance) = (unsafe { ctl.GetSessionInstanceIdentifier() })
            .ok()
            .and_then(|p| unsafe { take_pwstr(p) })
        else {
            continue;
        };
        let pid = unsafe { ctl.GetProcessId() }.unwrap_or(0);
        out.push(RawSession { ctl, instance, pid });
    }
    Ok(out)
}

pub(crate) fn sessions() -> Result<Vec<AudioSession>, AudioError> {
    let raw = raw_sessions()?;
    let mut sys = System::new();
    let pids: Vec<Pid> = raw.iter().map(|r| Pid::from_u32(r.pid)).collect();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing(),
    );
    let mut out = vec![];
    for r in raw {
        unsafe {
            let system_sounds = r.ctl.IsSystemSoundsSession() == S_OK;
            let vol = r.ctl.cast::<ISimpleAudioVolume>().ok();
            let meter = r.ctl.cast::<IAudioMeterInformation>().ok();
            let display = r
                .ctl
                .GetDisplayName()
                .ok()
                .and_then(|p| take_pwstr(p))
                .filter(|s| !s.is_empty() && !s.starts_with('@'));
            let app_name = if system_sounds {
                "System sounds".to_string()
            } else {
                sys.process(Pid::from_u32(r.pid))
                    .map(|p| {
                        p.name()
                            .to_string_lossy()
                            .trim_end_matches(".exe")
                            .to_string()
                    })
                    .unwrap_or_else(|| format!("PID {}", r.pid))
            };
            out.push(AudioSession {
                session_id: r.instance,
                pid: r.pid,
                app_name,
                display_name: display,
                active: r
                    .ctl
                    .GetState()
                    .map(|s| s == AudioSessionStateActive)
                    .unwrap_or(false),
                peak: meter.and_then(|m| m.GetPeakValue().ok()).unwrap_or(0.0),
                muted: vol
                    .as_ref()
                    .and_then(|v| v.GetMute().ok())
                    .map(|b| b.as_bool())
                    .unwrap_or(false),
                volume: vol
                    .as_ref()
                    .and_then(|v| v.GetMasterVolume().ok())
                    .unwrap_or(1.0),
                system_sounds,
                process_capture: false,
                process_capture_reason: None,
                can_silence_local: false,
            });
        }
    }
    Ok(out)
}

pub(crate) fn mutes_of(pid: u32) -> Result<Vec<(String, bool)>, AudioError> {
    Ok(raw_sessions()?
        .into_iter()
        .filter(|r| r.pid == pid)
        .filter_map(|r| {
            let v = r.ctl.cast::<ISimpleAudioVolume>().ok()?;
            Some((r.instance, unsafe { v.GetMute() }.ok()?.as_bool()))
        })
        .collect())
}

pub(crate) fn set_mute(instance: &str, muted: bool) -> Result<bool, AudioError> {
    for r in raw_sessions()? {
        if r.instance == instance {
            let v = r.ctl.cast::<ISimpleAudioVolume>().map_err(win)?;
            unsafe { v.SetMute(muted, std::ptr::null()) }.map_err(win)?;
            return Ok(true);
        }
    }
    Ok(false)
}
