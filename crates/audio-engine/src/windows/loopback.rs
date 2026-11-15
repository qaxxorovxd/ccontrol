//! Loopback capture: whole default output (all builds) or one process tree
//! (build 20348+, `ActivateAudioInterfaceAsync`).

use super::{format_48k_stereo_f32, sessions, win, Com};
use crate::device::AudioSource;
use crate::{AudioError, Frame, FRAME_LEN};
use ccontrol_protocol::CaptureKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::core::{implement, IUnknown, Interface, Ref, HRESULT};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{BLOB, CLSCTX_ALL};
use windows::Win32::System::Variant::VT_BLOB;

const BUFFER_HNS: i64 = 200_000; // 20 ms

enum Target {
    Endpoint,
    Process(u32),
}

struct Live {
    _com: Com,
    client: IAudioClient,
    capture: IAudioCaptureClient,
}

// SAFETY: the COM objects are created in the multithreaded apartment (MTA),
// whose objects may be called from any MTA thread; `Live` is only created
// and used by the sender thread that owns the source.
unsafe impl Send for Live {}

pub struct LoopbackSource {
    target: Target,
    live: Option<Live>,
    pending: Vec<f32>,
    /// Real-time clock for emitting silence when the device delivers nothing
    /// (Windows sends no loopback packets while nothing plays).
    next_due: Option<Instant>,
}

impl LoopbackSource {
    pub fn endpoint() -> Self {
        Self {
            target: Target::Endpoint,
            live: None,
            pending: vec![],
            next_due: None,
        }
    }
    pub fn process(pid: u32) -> Self {
        Self {
            target: Target::Process(pid),
            live: None,
            pending: vec![],
            next_due: None,
        }
    }

    fn open(&self) -> Result<Live, AudioError> {
        let com = Com::init();
        let client = match self.target {
            Target::Endpoint => {
                let dev = sessions::device(None)?;
                unsafe { dev.Activate::<IAudioClient>(CLSCTX_ALL, None) }.map_err(win)?
            }
            Target::Process(pid) => activate_process_loopback(pid)?,
        };
        let fmt = format_48k_stereo_f32();
        unsafe {
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    BUFFER_HNS,
                    0,
                    &fmt as *const _ as *const WAVEFORMATEX,
                    None,
                )
                .map_err(|e| AudioError::Device(format!("loopback init: {e}")))?;
            let capture: IAudioCaptureClient = client.GetService().map_err(win)?;
            client.Start().map_err(win)?;
            Ok(Live {
                _com: com,
                client,
                capture,
            })
        }
    }

    /// Drain whatever the device has buffered into `pending`. Returns true if any data arrived.
    fn drain(&mut self) -> Result<bool, AudioError> {
        let live = self.live.as_ref().unwrap();
        let mut got = false;
        unsafe {
            loop {
                let n = live
                    .capture
                    .GetNextPacketSize()
                    .map_err(|e| AudioError::Device(format!("capture device lost: {e}")))?;
                if n == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                live.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(win)?;
                let samples = frames as usize * 2;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    self.pending.extend(std::iter::repeat_n(0.0, samples));
                } else {
                    self.pending
                        .extend_from_slice(std::slice::from_raw_parts(data as *const f32, samples));
                }
                live.capture.ReleaseBuffer(frames).map_err(win)?;
                got = true;
            }
        }
        Ok(got)
    }
}

impl Drop for LoopbackSource {
    fn drop(&mut self) {
        if let Some(l) = &self.live {
            unsafe {
                let _ = l.client.Stop();
            }
        }
    }
}

impl AudioSource for LoopbackSource {
    fn kind(&self) -> CaptureKind {
        match self.target {
            Target::Endpoint => CaptureKind::Endpoint,
            Target::Process(_) => CaptureKind::Process,
        }
    }

    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, AudioError> {
        if self.live.is_none() {
            self.live = Some(self.open()?);
        }
        let due = *self.next_due.get_or_insert_with(Instant::now);
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            self.drain()?;
            if self.pending.len() >= FRAME_LEN {
                let f: Vec<f32> = self.pending.drain(..FRAME_LEN).collect();
                // Follow the device clock while it delivers data.
                self.next_due = Some(Instant::now().max(due) + Duration::from_millis(10));
                return Ok(Some(f));
            }
            // Device idle (nothing playing): keep the stream alive with silence
            // on our own 10 ms clock, without falling behind real time.
            if Instant::now() >= due + Duration::from_millis(15) {
                let mut f = std::mem::take(&mut self.pending);
                f.resize(FRAME_LEN, 0.0);
                self.next_due = Some(due + Duration::from_millis(10));
                return Ok(Some(f));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion(std::sync::mpsc::SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _op: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

fn activate_process_loopback(pid: u32) -> Result<IAudioClient, AudioError> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let pv = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: &mut params as *mut _ as *mut u8,
                    },
                },
            }),
        },
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Completion(tx).into();
    unsafe {
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&pv),
            &handler,
        )
        .map_err(|e| AudioError::Device(format!("process loopback activation: {e}")))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| AudioError::Device("process loopback activation timed out".into()))?;
        let mut hr = HRESULT(0);
        let mut unk: Option<IUnknown> = None;
        op.GetActivateResult(&mut hr, &mut unk).map_err(win)?;
        hr.ok()
            .map_err(|e| AudioError::Device(format!("process loopback: {e}")))?;
        unk.ok_or_else(|| AudioError::Device("no audio client".into()))?
            .cast()
            .map_err(win)
    }
}
