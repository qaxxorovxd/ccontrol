//! Sine test signal used to verify a route end to end.

use crate::{Frame, FRAME_SAMPLES_PER_CH};

pub struct Tone {
    phase: f64,
    step: f64,
    gain: f32,
}

impl Tone {
    pub fn new(freq_hz: f64, gain: f32) -> Self {
        Self {
            phase: 0.0,
            step: freq_hz * std::f64::consts::TAU / 48_000.0,
            gain: gain.clamp(0.0, 1.0),
        }
    }

    pub fn next_frame(&mut self) -> Frame {
        let mut f = Vec::with_capacity(FRAME_SAMPLES_PER_CH * 2);
        for _ in 0..FRAME_SAMPLES_PER_CH {
            let s = (self.phase.sin() as f32) * self.gain;
            f.push(s);
            f.push(s);
            self.phase = (self.phase + self.step) % std::f64::consts::TAU;
        }
        f
    }
}
