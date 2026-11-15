//! Adaptive jitter buffer with Opus FEC/PLC and simple drift control.
//!
//! * Prebuffers `target` frames before playout starts.
//! * Target depth adapts to measured interarrival jitter (RFC 3550 estimator):
//!   `target = clamp(2 + ceil(2·jitter / 10 ms), 2, 12)` frames (20–120 ms).
//! * Missing frame: recover from the next packet's inband FEC if present,
//!   otherwise Opus packet-loss concealment.
//! * Clock drift between sender and receiver shows up as a slowly growing or
//!   shrinking queue; when the queue exceeds `target + 4` one frame is
//!   dropped, when it runs dry the decoder conceals and playout rebuffers.

use crate::codec::Decoder;
use crate::{Frame, FRAME_LEN};
use std::collections::BTreeMap;

const MAX_QUEUE: usize = 64;
const MIN_TARGET: usize = 2;
const MAX_TARGET: usize = 12;
const REBUFFER_AFTER_UNDERRUNS: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    Accepted,
    Late,
    Duplicate,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JitterStats {
    pub packets: u64,
    pub lost: u64,
    pub late: u64,
    pub duplicates: u64,
    pub fec_recovered: u64,
    pub concealed: u64,
    pub dropped_for_drift: u64,
    pub underruns: u64,
    pub frames_played: u64,
    pub jitter_us: f64,
    pub depth_frames: usize,
    pub target_frames: usize,
    pub last_peak: f32,
}

pub struct JitterBuffer {
    queue: BTreeMap<u64, Vec<u8>>,
    next_seq: Option<u64>,
    started: bool,
    target: usize,
    consecutive_underruns: u32,
    last_transit: Option<i64>,
    decoder: Decoder,
    pub stats: JitterStats,
}

impl JitterBuffer {
    pub fn new(decoder: Decoder) -> Self {
        Self {
            queue: BTreeMap::new(),
            next_seq: None,
            started: false,
            target: 3,
            consecutive_underruns: 0,
            last_transit: None,
            decoder,
            stats: JitterStats {
                target_frames: 3,
                ..Default::default()
            },
        }
    }

    /// `send_us` is the sender's clock, `arrival_us` the local clock; only
    /// their differences matter for jitter.
    pub fn push(&mut self, seq: u64, send_us: u64, arrival_us: u64, payload: Vec<u8>) -> Push {
        if let Some(n) = self.next_seq {
            if self.started && seq < n {
                self.stats.late += 1;
                return Push::Late;
            }
        }
        if self.queue.contains_key(&seq) {
            self.stats.duplicates += 1;
            return Push::Duplicate;
        }
        self.stats.packets += 1;
        let transit = arrival_us as i64 - send_us as i64;
        if let Some(prev) = self.last_transit {
            let d = (transit - prev).unsigned_abs() as f64;
            self.stats.jitter_us += (d - self.stats.jitter_us) / 16.0;
        }
        self.last_transit = Some(transit);
        self.target = (MIN_TARGET + ((2.0 * self.stats.jitter_us) / 10_000.0).ceil() as usize)
            .clamp(MIN_TARGET, MAX_TARGET);
        self.stats.target_frames = self.target;
        self.queue.insert(seq, payload);
        while self.queue.len() > MAX_QUEUE {
            let first = *self.queue.keys().next().unwrap();
            self.queue.remove(&first);
            if self.started {
                self.next_seq = Some(first + 1);
            }
        }
        self.stats.depth_frames = self.queue.len();
        Push::Accepted
    }

    fn silence() -> Frame {
        vec![0.0; FRAME_LEN]
    }

    /// Produce the next 10 ms frame for the device.
    pub fn pop(&mut self) -> Frame {
        if !self.started {
            if self.queue.len() >= self.target {
                self.started = true;
                self.next_seq = self.queue.keys().next().copied();
            } else {
                return Self::silence();
            }
        }
        // Drift control: queue far above target -> drop the oldest frame.
        if self.queue.len() > self.target + 4 {
            if let Some(first) = self.queue.keys().next().copied() {
                self.queue.remove(&first);
                self.next_seq = Some(first + 1);
                self.stats.dropped_for_drift += 1;
            }
        }
        let seq = self.next_seq.unwrap_or(0);
        let frame = if let Some(p) = self.queue.remove(&seq) {
            self.consecutive_underruns = 0;
            self.decoder
                .decode(&p)
                .unwrap_or_else(|_| self.decoder.conceal())
        } else if self.queue.is_empty() {
            self.stats.underruns += 1;
            self.consecutive_underruns += 1;
            if self.consecutive_underruns >= REBUFFER_AFTER_UNDERRUNS {
                // Stream paused or network stalled: rebuffer instead of concealing forever.
                self.started = false;
                self.consecutive_underruns = 0;
                return Self::silence();
            }
            self.stats.concealed += 1;
            self.decoder.conceal()
        } else {
            self.stats.lost += 1;
            self.consecutive_underruns = 0;
            match self.queue.get(&(seq + 1)) {
                Some(next) => {
                    self.stats.fec_recovered += 1;
                    let next = next.clone();
                    self.decoder
                        .decode_fec(&next)
                        .unwrap_or_else(|_| self.decoder.conceal())
                }
                None => {
                    self.stats.concealed += 1;
                    self.decoder.conceal()
                }
            }
        };
        self.next_seq = Some(seq + 1);
        self.stats.frames_played += 1;
        self.stats.depth_frames = self.queue.len();
        self.stats.last_peak = crate::peak(&frame);
        frame
    }

    pub fn buffer_ms(&self) -> f32 {
        self.queue.len() as f32 * 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Encoder;
    use crate::tone::Tone;

    fn packets(n: usize) -> Vec<Vec<u8>> {
        let mut e = Encoder::new(96_000).unwrap();
        let mut t = Tone::new(440.0, 0.5);
        (0..n).map(|_| e.encode(&t.next_frame()).unwrap()).collect()
    }

    #[test]
    fn prebuffers_then_plays_in_order() {
        let mut jb = JitterBuffer::new(Decoder::new().unwrap());
        let p = packets(10);
        jb.push(0, 0, 1000, p[0].clone());
        assert_eq!(crate::peak(&jb.pop()), 0.0, "still prebuffering");
        for (i, pk) in p.iter().enumerate().skip(1) {
            jb.push(
                i as u64,
                i as u64 * 10_000,
                1000 + i as u64 * 10_000,
                pk.clone(),
            );
        }
        for _ in 0..10 {
            jb.pop();
        }
        assert_eq!(jb.stats.frames_played, 10);
        assert_eq!(jb.stats.lost, 0);
        assert!(jb.stats.last_peak > 0.2);
    }

    #[test]
    fn reorders_recovers_loss_and_rejects_late() {
        let mut jb = JitterBuffer::new(Decoder::new().unwrap());
        let p = packets(12);
        // deliver 0,1,2,4,3,6 (5 lost) ...
        for &i in &[0usize, 1, 2, 4, 3, 6, 7, 8, 9] {
            jb.push(i as u64, i as u64 * 10_000, i as u64 * 10_000, p[i].clone());
        }
        for _ in 0..9 {
            jb.pop();
        }
        assert_eq!(jb.stats.lost, 1, "{:?}", jb.stats);
        assert_eq!(jb.stats.fec_recovered, 1, "seq 6 carries FEC for 5");
        assert_eq!(jb.push(5, 0, 0, p[5].clone()), Push::Late);
        assert_eq!(jb.push(9, 0, 0, p[9].clone()), Push::Late);
    }

    #[test]
    fn duplicates_are_counted() {
        let mut jb = JitterBuffer::new(Decoder::new().unwrap());
        let p = packets(2);
        assert_eq!(jb.push(0, 0, 0, p[0].clone()), Push::Accepted);
        assert_eq!(jb.push(0, 0, 0, p[0].clone()), Push::Duplicate);
    }

    #[test]
    fn jitter_raises_target_and_drift_drops_frames() {
        let mut jb = JitterBuffer::new(Decoder::new().unwrap());
        let p = packets(40);
        for (i, pk) in p.iter().enumerate() {
            // ±15 ms arrival noise
            let noise = if i % 2 == 0 { 15_000 } else { 0 };
            jb.push(
                i as u64,
                i as u64 * 10_000,
                i as u64 * 10_000 + noise,
                pk.clone(),
            );
        }
        assert!(jb.stats.target_frames > 3, "{:?}", jb.stats);
        jb.pop();
        assert!(
            jb.stats.dropped_for_drift > 0,
            "queue of 40 far above target"
        );
    }

    #[test]
    fn underruns_then_rebuffer() {
        let mut jb = JitterBuffer::new(Decoder::new().unwrap());
        let p = packets(3);
        for (i, pk) in p.iter().enumerate() {
            jb.push(i as u64, 0, 0, pk.clone());
        }
        for _ in 0..30 {
            jb.pop();
        }
        assert!(jb.stats.underruns >= 20);
        assert!(!jb.started, "rebuffering after a long stall");
    }
}
