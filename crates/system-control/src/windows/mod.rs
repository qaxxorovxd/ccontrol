//! Windows adapters: Core Audio (IAudioEndpointVolume) and media keys via SendInput.

use crate::{check_level, ControlError, MediaControl, VolumeControl};
use ccontrol_protocol::{MediaKey, VolumeState};
use windows::core::{GUID, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP,
};

fn plat(e: windows::core::Error) -> ControlError {
    ControlError::Platform(format!("{e}"))
}

/// COM apartment guard for the current (blocking pool) thread.
struct Com(bool);
impl Com {
    fn init() -> Self {
        // S_OK or S_FALSE both require a matching CoUninitialize.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Com(hr.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

pub struct CoreAudioVolume;

impl CoreAudioVolume {
    fn with<T>(
        &self,
        f: impl FnOnce(&IAudioEndpointVolume) -> windows::core::Result<T>,
    ) -> Result<(T, VolumeState), ControlError> {
        let _com = Com::init();
        unsafe {
            let en: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(plat)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| {
                ControlError::Unsupported(format!("no default playback device: {e}"))
            })?;
            let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).map_err(plat)?;
            let out = f(&vol).map_err(plat)?;
            let state = VolumeState {
                level: vol.GetMasterVolumeLevelScalar().map_err(plat)?,
                muted: vol.GetMute().map_err(plat)?.as_bool(),
                endpoint_name: friendly_name(&dev),
            };
            Ok((out, state))
        }
    }
}

unsafe fn friendly_name(dev: &IMMDevice) -> Option<String> {
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let pv = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
    let p: PWSTR = PropVariantToStringAlloc(&pv).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

impl VolumeControl for CoreAudioVolume {
    fn get(&self) -> Result<VolumeState, ControlError> {
        self.with(|_| Ok(())).map(|(_, s)| s)
    }

    fn set_level(&self, level: f32) -> Result<VolumeState, ControlError> {
        let level = check_level(level)?;
        self.with(|v| unsafe { v.SetMasterVolumeLevelScalar(level, &GUID::zeroed()) })
            .map(|(_, s)| s)
    }

    fn set_muted(&self, muted: bool) -> Result<VolumeState, ControlError> {
        self.with(|v| unsafe { v.SetMute(muted, &GUID::zeroed()) })
            .map(|(_, s)| s)
    }
}

pub struct MediaKeys;

impl MediaControl for MediaKeys {
    fn send(&self, key: MediaKey) -> Result<(), ControlError> {
        let vk: VIRTUAL_KEY = match key {
            MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
            MediaKey::Next => VK_MEDIA_NEXT_TRACK,
            MediaKey::Previous => VK_MEDIA_PREV_TRACK,
            MediaKey::Stop => VK_MEDIA_STOP,
        };
        let mk = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [mk(Default::default()), mk(KEYEVENTF_KEYUP)];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            // Typical cause: the agent runs outside the interactive desktop
            // (Session 0 / secure desktop / UIPI).
            Err(ControlError::Platform(format!(
                "SendInput injected {sent}/2 events: {}",
                windows::core::Error::from_win32()
            )))
        }
    }
}
