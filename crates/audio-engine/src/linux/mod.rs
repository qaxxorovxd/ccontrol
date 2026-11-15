//! PipeWire backend via the stock `pw-dump` / `pw-record` / `pw-play` tools
//! (fixed executables, typed arguments, raw f32 PCM over pipes).
//!
//! Status: compiled and unit-tested (graph parser) but **not yet run on a
//! Linux machine**. Native libpipewire bindings are planned. Local silencing
//! (moving the app's stream to a null sink) is not implemented, so Linux
//! routes run in capture-and-forward mode and keep local playback.

use crate::device::{AudioSink, AudioSource, AudioSystem, ToneSource};
use crate::pwdump;
use crate::{AudioError, Frame, FRAME_LEN};
use ccontrol_protocol::{AudioInventory, CaptureKind, RouteSource};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub struct PipeWireSystem;

fn graph() -> Result<pwdump::PwGraph, AudioError> {
    let out = Command::new("pw-dump")
        .output()
        .map_err(|e| AudioError::Unsupported(format!("PipeWire tools not found (pw-dump): {e}")))?;
    if !out.status.success() {
        return Err(AudioError::Unsupported(
            "pw-dump failed: is PipeWire running for this user?".into(),
        ));
    }
    pwdump::parse(&String::from_utf8_lossy(&out.stdout)).map_err(AudioError::Device)
}

const FMT: [&str; 6] = ["--rate", "48000", "--channels", "2", "--format", "f32"];

impl AudioSystem for PipeWireSystem {
    fn inventory(&self) -> Result<AudioInventory, AudioError> {
        let g = graph()?;
        Ok(AudioInventory {
            can_play: !g.sinks.is_empty(),
            endpoint_capture: !g.sinks.is_empty(),
            endpoints: g.sinks,
            sessions: g.streams.into_iter().map(|(_, s)| s).collect(),
            process_capture: true,
            process_capture_reason: None,
        })
    }

    fn open_source(&self, src: &RouteSource) -> Result<Box<dyn AudioSource>, AudioError> {
        match src {
            RouteSource::TestTone => Ok(Box::new(ToneSource::new(0.15))),
            RouteSource::Endpoint => {
                let g = graph()?;
                let sink = g
                    .sinks
                    .iter()
                    .find(|s| s.is_default)
                    .or(g.sinks.first())
                    .ok_or_else(|| AudioError::Unsupported("no output device".into()))?;
                // Record the sink's monitor (everything it plays).
                PwRecord::spawn(&sink.id, true, CaptureKind::Endpoint).map(|s| Box::new(s) as _)
            }
            RouteSource::Session { pid, .. } => {
                let g = graph()?;
                let (node, _) = g
                    .streams
                    .iter()
                    .find(|(_, s)| s.pid == *pid)
                    .ok_or_else(|| {
                        AudioError::Invalid(format!("process {pid} has no audio stream"))
                    })?;
                PwRecord::spawn(&node.to_string(), false, CaptureKind::Process)
                    .map(|s| Box::new(s) as _)
            }
        }
    }

    fn open_sink(&self, endpoint_id: Option<&str>) -> Result<Box<dyn AudioSink>, AudioError> {
        let g = graph()?;
        let ep = match endpoint_id {
            Some(id) => g.sinks.iter().find(|s| s.id == id),
            None => g.sinks.iter().find(|s| s.is_default).or(g.sinks.first()),
        }
        .ok_or_else(|| AudioError::Invalid("output device not found".into()))?;
        Ok(Box::new(PwPlay {
            target: ep.id.clone(),
            name: ep.name.clone(),
        }))
    }

    fn session_mutes(&self, _pid: u32) -> Result<Vec<(String, bool)>, AudioError> {
        Ok(vec![])
    }

    fn set_session_mute(&self, _: &str, _: bool) -> Result<bool, AudioError> {
        Ok(false)
    }

    fn silence_local_supported(&self, _: CaptureKind) -> Result<(), String> {
        Err(
            "Silencing one app locally is not implemented on Linux yet; local playback is kept."
                .into(),
        )
    }
}

struct PwRecord {
    child: Child,
    out: ChildStdout,
    kind: CaptureKind,
}

impl PwRecord {
    fn spawn(target: &str, monitor: bool, kind: CaptureKind) -> Result<Self, AudioError> {
        let mut cmd = Command::new("pw-record");
        cmd.args(["--target", target, "--latency", "10ms"])
            .args(FMT);
        if monitor {
            cmd.args(["-P", "{ stream.capture.sink=true }"]);
        }
        let mut child = cmd
            .arg("-")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AudioError::Unsupported(format!("pw-record: {e}")))?;
        let out = child.stdout.take().unwrap();
        Ok(Self { child, out, kind })
    }
}

impl Drop for PwRecord {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AudioSource for PwRecord {
    fn kind(&self) -> CaptureKind {
        self.kind
    }
    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, AudioError> {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut bytes = vec![0u8; FRAME_LEN * 4];
        self.out
            .read_exact(&mut bytes)
            .map_err(|e| AudioError::Device(format!("capture ended: {e}")))?;
        Ok(Some(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect(),
        ))
    }
}

struct PwPlay {
    target: String,
    name: String,
}

impl AudioSink for PwPlay {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn device_latency_ms(&self) -> f32 {
        20.0
    }
    fn run(
        &mut self,
        next: &mut dyn FnMut() -> Frame,
        stop: &AtomicBool,
    ) -> Result<(), AudioError> {
        let mut child = Command::new("pw-play")
            .args(["--target", &self.target, "--latency", "20ms"])
            .args(FMT)
            .arg("-")
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AudioError::Unsupported(format!("pw-play: {e}")))?;
        let mut stdin: ChildStdin = child.stdin.take().unwrap();
        let mut deadline = Instant::now();
        let result = (|| {
            while !stop.load(Ordering::Relaxed) {
                let f = next();
                let bytes: Vec<u8> = f.iter().flat_map(|s| s.to_le_bytes()).collect();
                stdin
                    .write_all(&bytes)
                    .map_err(|e| AudioError::Device(format!("output ended: {e}")))?;
                deadline += Duration::from_millis(10);
                let now = Instant::now();
                if deadline > now {
                    std::thread::sleep(deadline - now);
                }
            }
            Ok(())
        })();
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        result
    }
}
