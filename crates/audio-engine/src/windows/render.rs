//! Shared-mode WASAPI render of 48 kHz stereo float frames.

use super::{format_48k_stereo_f32, sessions, win, Com};
use crate::device::AudioSink;
use crate::{AudioError, Frame, FRAME_SAMPLES_PER_CH};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Win32::Media::Audio::{
    IAudioClient, IAudioRenderClient, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::CLSCTX_ALL;

/// Device buffer requested from WASAPI (100 ns units): 30 ms.
const BUFFER_HNS: i64 = 300_000;

pub struct RenderSink {
    endpoint_id: Option<String>,
    name: String,
}

impl RenderSink {
    pub fn new(endpoint_id: Option<String>, name: String) -> Self {
        Self { endpoint_id, name }
    }
}

impl AudioSink for RenderSink {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn device_latency_ms(&self) -> f32 {
        (BUFFER_HNS / 10_000) as f32
    }

    fn run(
        &mut self,
        next: &mut dyn FnMut() -> Frame,
        stop: &AtomicBool,
    ) -> Result<(), AudioError> {
        let _com = Com::init();
        let dev = sessions::device(self.endpoint_id.as_deref())?;
        unsafe {
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(win)?;
            let fmt = format_48k_stereo_f32();
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    BUFFER_HNS,
                    0,
                    &fmt as *const _ as *const WAVEFORMATEX,
                    None,
                )
                .map_err(|e| AudioError::Device(format!("render init: {e}")))?;
            let buffer_frames = client.GetBufferSize().map_err(win)?;
            let rc: IAudioRenderClient = client.GetService().map_err(win)?;
            client.Start().map_err(win)?;
            let per = FRAME_SAMPLES_PER_CH as u32;
            let result = (|| -> Result<(), AudioError> {
                while !stop.load(Ordering::Relaxed) {
                    let padding = client
                        .GetCurrentPadding()
                        .map_err(|e| AudioError::Device(format!("output device lost: {e}")))?;
                    let mut avail = buffer_frames.saturating_sub(padding);
                    while avail >= per {
                        let f = next();
                        let ptr = rc.GetBuffer(per).map_err(win)? as *mut f32;
                        std::ptr::copy_nonoverlapping(
                            f.as_ptr(),
                            ptr,
                            f.len().min(per as usize * 2),
                        );
                        rc.ReleaseBuffer(per, 0).map_err(win)?;
                        avail -= per;
                    }
                    std::thread::sleep(Duration::from_millis(3));
                }
                Ok(())
            })();
            let _ = client.Stop();
            result
        }
    }
}
