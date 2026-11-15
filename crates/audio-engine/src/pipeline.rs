//! Sender and receiver threads for one route.

use crate::codec::{Decoder, Encoder};
use crate::device::{AudioSink, AudioSource};
use crate::jitter::JitterBuffer;
use crate::packet::{Kind, ReplayWindow, Report, StreamKey};
use crate::AudioError;
use ccontrol_protocol::{CaptureKind, StreamStats};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const BITRATE: i32 = 128_000;
const REPORT_EVERY: Duration = Duration::from_millis(500);

/// Monotonic microseconds since process start (the "sender clock").
pub fn now_us() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------- sender

#[derive(Default)]
struct SenderShared {
    frames: u64,
    bytes: u64,
    started: Option<Instant>,
    peak: f32,
    /// Max level since the last `capture_peak()` call.
    window_peak: f32,
    report: Option<Report>,
    report_at: Option<Instant>,
    rtt_ms: Option<f32>,
    error: Option<String>,
    finished: bool,
}

pub struct SenderHandle {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<SenderShared>>,
    threads: Vec<JoinHandle<()>>,
    kind: CaptureKind,
}

impl SenderHandle {
    pub fn start(
        mut source: Box<dyn AudioSource>,
        dest: SocketAddr,
        key: Arc<StreamKey>,
    ) -> Result<Self, AudioError> {
        let bind: SocketAddr = if dest.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };
        let sock = UdpSocket::bind(bind).map_err(|e| AudioError::Net(e.to_string()))?;
        sock.connect(dest)
            .map_err(|e| AudioError::Net(format!("connect {dest}: {e}")))?;
        let rx = sock
            .try_clone()
            .map_err(|e| AudioError::Net(e.to_string()))?;
        rx.set_read_timeout(Some(Duration::from_millis(200))).ok();
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(SenderShared {
            started: Some(Instant::now()),
            ..Default::default()
        }));
        let kind = source.kind();

        let (st, sh, k) = (stop.clone(), shared.clone(), key.clone());
        let send = std::thread::Builder::new()
            .name("ccontrol-audio-send".into())
            .spawn(move || {
                let mut enc = match Encoder::new(BITRATE) {
                    Ok(e) => e,
                    Err(e) => {
                        lock(&sh).error = Some(e.to_string());
                        return;
                    }
                };
                let mut seq = 0u64;
                loop {
                    match source.read_frame(&st) {
                        Ok(Some(frame)) => {
                            let p = crate::peak(&frame);
                            let pkt = enc
                                .encode(&frame)
                                .and_then(|op| k.seal(Kind::Media, seq, now_us(), &op));
                            match pkt {
                                Ok(pkt) => {
                                    // Errors here are typically ICMP "port unreachable" echoes
                                    // while the destination restarts; keep streaming.
                                    let _ = sock.send(&pkt);
                                    let mut g = lock(&sh);
                                    g.frames += 1;
                                    g.bytes += pkt.len() as u64;
                                    g.peak = p.max(g.peak * 0.95);
                                    g.window_peak = g.window_peak.max(p);
                                }
                                Err(e) => {
                                    lock(&sh).error = Some(e.to_string());
                                    break;
                                }
                            }
                            seq += 1;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            lock(&sh).error = Some(e.to_string());
                            break;
                        }
                    }
                }
                lock(&sh).finished = true;
            })
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let (st, sh) = (stop.clone(), shared.clone());
        let reports = std::thread::Builder::new()
            .name("ccontrol-audio-rr".into())
            .spawn(move || {
                let mut buf = [0u8; 1500];
                let mut window = ReplayWindow::default();
                while !st.load(Ordering::Relaxed) {
                    let Ok(n) = rx.recv(&mut buf) else { continue };
                    let Some((h, body)) = key.open(&buf[..n]) else {
                        continue;
                    };
                    if h.kind != Kind::Report || !window.accept(h.seq) {
                        continue;
                    }
                    if let Some(r) = Report::decode(&body) {
                        let rtt_us = now_us()
                            .saturating_sub(r.echo_send_time_us)
                            .saturating_sub(r.hold_us as u64);
                        let mut g = lock(&sh);
                        g.rtt_ms = Some(rtt_us as f32 / 1000.0);
                        g.report = Some(r);
                        g.report_at = Some(Instant::now());
                    }
                }
            })
            .map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            stop,
            shared,
            threads: vec![send, reports],
            kind,
        })
    }

    pub fn kind(&self) -> CaptureKind {
        self.kind
    }

    /// Sender-side stats merged with the latest receiver report.
    pub fn stats(&self) -> StreamStats {
        let g = lock(&self.shared);
        let secs = g
            .started
            .map(|s| s.elapsed().as_secs_f32())
            .unwrap_or(0.0)
            .max(0.001);
        let fresh = g
            .report_at
            .map(|t| t.elapsed() < Duration::from_secs(3))
            .unwrap_or(false);
        let r = g.report.filter(|_| fresh).unwrap_or_default();
        let buffer_ms = r.buffer_us as f32 / 1000.0;
        StreamStats {
            packets: g.frames,
            lost: r.lost,
            late: r.late,
            duplicates: 0,
            concealed_frames: 0,
            frames_played: r.frames_played,
            underruns: r.underruns as u64,
            jitter_ms: r.jitter_us as f32 / 1000.0,
            buffer_ms,
            rtt_ms: g.rtt_ms.filter(|_| fresh),
            latency_ms: g
                .rtt_ms
                .filter(|_| fresh)
                .map(|rtt| rtt / 2.0 + buffer_ms + 10.0),
            bitrate_kbps: g.bytes as f32 * 8.0 / 1000.0 / secs,
            peak: g.peak,
        }
    }

    /// Latest receiver report, if one arrived in the last 3 s.
    pub fn receiver_report(&self) -> Option<Report> {
        let g = lock(&self.shared);
        g.report.filter(|_| {
            g.report_at
                .map(|t| t.elapsed() < Duration::from_secs(3))
                .unwrap_or(false)
        })
    }

    /// Highest capture level since the previous call (resets the window).
    pub fn capture_peak(&self) -> f32 {
        std::mem::take(&mut lock(&self.shared).window_peak)
    }

    pub fn error(&self) -> Option<String> {
        lock(&self.shared).error.clone()
    }

    pub fn is_running(&self) -> bool {
        !lock(&self.shared).finished
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for SenderHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------- receiver

struct RecvShared {
    jb: JitterBuffer,
    peer: Option<SocketAddr>,
    newest_send_us: u64,
    newest_arrival_us: u64,
    last_packet: Option<Instant>,
    bytes: u64,
    auth_failures: u64,
    started: Instant,
    device_latency_ms: f32,
    error: Option<String>,
    render_running: bool,
}

pub struct ReceiverHandle {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<RecvShared>>,
    port: u16,
    sink_name: String,
    threads: Vec<JoinHandle<()>>,
}

impl ReceiverHandle {
    pub fn start(key: Arc<StreamKey>, mut sink: Box<dyn AudioSink>) -> Result<Self, AudioError> {
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| AudioError::Net(e.to_string()))?;
        let port = sock
            .local_addr()
            .map_err(|e| AudioError::Net(e.to_string()))?
            .port();
        sock.set_read_timeout(Some(Duration::from_millis(100))).ok();
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(RecvShared {
            jb: JitterBuffer::new(Decoder::new()?),
            peer: None,
            newest_send_us: 0,
            newest_arrival_us: 0,
            last_packet: None,
            bytes: 0,
            auth_failures: 0,
            started: Instant::now(),
            device_latency_ms: sink.device_latency_ms(),
            error: None,
            render_running: true,
        }));
        let sink_name = sink.name();

        let (st, sh) = (stop.clone(), shared.clone());
        let net = std::thread::Builder::new()
            .name("ccontrol-audio-recv".into())
            .spawn(move || {
                let mut buf = [0u8; 1500];
                let mut window = ReplayWindow::default();
                let mut report_seq = 0u64;
                let mut last_report = Instant::now();
                while !st.load(Ordering::Relaxed) {
                    if let Ok((n, from)) = sock.recv_from(&mut buf) {
                        match key.open(&buf[..n]) {
                            Some((h, body)) if h.kind == Kind::Media && window.accept(h.seq) => {
                                let arrival = now_us();
                                let mut g = lock(&sh);
                                g.peer = Some(from);
                                g.bytes += n as u64;
                                g.last_packet = Some(Instant::now());
                                if h.send_time_us >= g.newest_send_us {
                                    g.newest_send_us = h.send_time_us;
                                    g.newest_arrival_us = arrival;
                                }
                                g.jb.push(h.seq, h.send_time_us, arrival, body);
                            }
                            Some(_) => {}
                            None => lock(&sh).auth_failures += 1,
                        }
                    }
                    if last_report.elapsed() >= REPORT_EVERY {
                        last_report = Instant::now();
                        let (peer, report) = {
                            let g = lock(&sh);
                            let s = &g.jb.stats;
                            (
                                g.peer,
                                Report {
                                    frames_played: s.frames_played,
                                    packets: s.packets,
                                    lost: s.lost,
                                    late: s.late,
                                    echo_send_time_us: g.newest_send_us,
                                    hold_us: now_us()
                                        .saturating_sub(g.newest_arrival_us)
                                        .min(u32::MAX as u64)
                                        as u32,
                                    jitter_us: s.jitter_us as u32,
                                    buffer_us: (g.jb.buffer_ms() * 1000.0) as u32
                                        + (g.device_latency_ms * 1000.0) as u32,
                                    peak_milli: (s.last_peak * 1000.0) as u16,
                                    underruns: s.underruns.min(u16::MAX as u64) as u16,
                                },
                            )
                        };
                        if let Some(peer) = peer {
                            if let Ok(p) =
                                key.seal(Kind::Report, report_seq, now_us(), &report.encode())
                            {
                                let _ = sock.send_to(&p, peer);
                                report_seq += 1;
                            }
                        }
                    }
                }
            })
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let (st, sh) = (stop.clone(), shared.clone());
        let render = std::thread::Builder::new()
            .name("ccontrol-audio-render".into())
            .spawn(move || {
                let sh2 = sh.clone();
                let mut next = move || lock(&sh2).jb.pop();
                let r = sink.run(&mut next, &st);
                let mut g = lock(&sh);
                g.render_running = false;
                if let Err(e) = r {
                    g.error = Some(e.to_string());
                }
            })
            .map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            stop,
            shared,
            port,
            sink_name,
            threads: vec![net, render],
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn sink_name(&self) -> &str {
        &self.sink_name
    }

    /// Has real (non-prebuffer) audio been rendered and is the stream current?
    pub fn playing(&self) -> bool {
        let g = lock(&self.shared);
        g.jb.stats.frames_played > 0
            && g.last_packet
                .map(|t| t.elapsed() < Duration::from_secs(1))
                .unwrap_or(false)
    }

    pub fn error(&self) -> Option<String> {
        let g = lock(&self.shared);
        g.error
            .clone()
            .or_else(|| (!g.render_running).then(|| "audio output stopped".to_string()))
    }

    pub fn auth_failures(&self) -> u64 {
        lock(&self.shared).auth_failures
    }

    pub fn stats(&self) -> StreamStats {
        let g = lock(&self.shared);
        let s = &g.jb.stats;
        let secs = g.started.elapsed().as_secs_f32().max(0.001);
        let buffer_ms = g.jb.buffer_ms() + g.device_latency_ms;
        StreamStats {
            packets: s.packets,
            lost: s.lost,
            late: s.late,
            duplicates: s.duplicates,
            concealed_frames: s.concealed,
            frames_played: s.frames_played,
            underruns: s.underruns,
            jitter_ms: (s.jitter_us / 1000.0) as f32,
            buffer_ms,
            rtt_ms: None,
            latency_ms: None,
            bitrate_kbps: g.bytes as f32 * 8.0 / 1000.0 / secs,
            peak: s.last_peak,
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for ReceiverHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{MemorySink, ToneSource};
    use crate::packet::random_key_hex;

    #[test]
    fn tone_streams_end_to_end_over_udp() {
        let k = random_key_hex();
        let sink = MemorySink::new();
        let peaks = sink.peaks.clone();
        let rx = ReceiverHandle::start(Arc::new(StreamKey::new("r", &k).unwrap()), Box::new(sink))
            .unwrap();
        let dest: SocketAddr = format!("127.0.0.1:{}", rx.port()).parse().unwrap();
        let tx = SenderHandle::start(
            Box::new(ToneSource::new(0.5)),
            dest,
            Arc::new(StreamKey::new("r", &k).unwrap()),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        assert!(rx.playing());
        let rs = rx.stats();
        assert!(rs.frames_played > 50, "{rs:?}");
        assert_eq!(rs.lost, 0, "{rs:?}");
        assert!(
            peaks.lock().unwrap().iter().any(|p| *p > 0.3),
            "decoded tone is audible"
        );
        let ss = tx.stats();
        assert!(
            ss.rtt_ms.is_some(),
            "receiver reports reached the sender: {ss:?}"
        );
        assert!(ss.latency_ms.unwrap() < 200.0, "{ss:?}");
        assert!(ss.bitrate_kbps > 50.0 && ss.bitrate_kbps < 250.0, "{ss:?}");
        tx.stop();
        rx.stop();
    }

    #[test]
    fn receiver_ignores_wrong_key() {
        let rx = ReceiverHandle::start(
            Arc::new(StreamKey::new("r", &random_key_hex()).unwrap()),
            Box::new(MemorySink::new()),
        )
        .unwrap();
        let dest: SocketAddr = format!("127.0.0.1:{}", rx.port()).parse().unwrap();
        let tx = SenderHandle::start(
            Box::new(ToneSource::new(0.5)),
            dest,
            Arc::new(StreamKey::new("r", &random_key_hex()).unwrap()),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(!rx.playing());
        assert!(rx.auth_failures() > 10);
        assert_eq!(rx.stats().packets, 0);
        drop(tx);
    }
}
