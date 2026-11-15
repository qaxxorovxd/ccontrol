//! Opus wrapper: 48 kHz stereo, 10 ms frames, low-delay audio mode.

use crate::{AudioError, FRAME_LEN, FRAME_SAMPLES_PER_CH};

pub struct Encoder(opus::Encoder);
pub struct Decoder(opus::Decoder);

fn err(e: opus::Error) -> AudioError {
    AudioError::Codec(e.to_string())
}

impl Encoder {
    pub fn new(bitrate: i32) -> Result<Self, AudioError> {
        let mut e = opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::LowDelay)
            .map_err(err)?;
        e.set_bitrate(opus::Bitrate::Bits(bitrate)).map_err(err)?;
        // Inband FEC lets the decoder recover a single lost packet.
        e.set_inband_fec(true).map_err(err)?;
        e.set_packet_loss_perc(5).map_err(err)?;
        Ok(Self(e))
    }

    pub fn encode(&mut self, frame: &[f32]) -> Result<Vec<u8>, AudioError> {
        if frame.len() != FRAME_LEN {
            return Err(AudioError::Invalid(format!(
                "frame has {} samples",
                frame.len()
            )));
        }
        let mut out = vec![0u8; 1200];
        let n = self.0.encode_float(frame, &mut out).map_err(err)?;
        out.truncate(n);
        Ok(out)
    }
}

impl Decoder {
    pub fn new() -> Result<Self, AudioError> {
        Ok(Self(
            opus::Decoder::new(48_000, opus::Channels::Stereo).map_err(err)?,
        ))
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>, AudioError> {
        let mut out = vec![0f32; FRAME_LEN];
        let n = self.0.decode_float(packet, &mut out, false).map_err(err)?;
        out.truncate(n * 2);
        out.resize(FRAME_LEN, 0.0);
        Ok(out)
    }

    /// Recover a lost frame from the FEC data carried in the *next* packet.
    pub fn decode_fec(&mut self, next_packet: &[u8]) -> Result<Vec<f32>, AudioError> {
        let mut out = vec![0f32; FRAME_LEN];
        let n = self
            .0
            .decode_float(next_packet, &mut out, true)
            .map_err(err)?;
        out.truncate(n * 2);
        out.resize(FRAME_LEN, 0.0);
        Ok(out)
    }

    /// Packet loss concealment for a missing frame.
    pub fn conceal(&mut self) -> Vec<f32> {
        let mut out = vec![0f32; FRAME_LEN];
        match self.0.decode_float(&[], &mut out, false) {
            Ok(n) if n == FRAME_SAMPLES_PER_CH => out,
            _ => vec![0.0; FRAME_LEN],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tone::Tone;

    #[test]
    fn encode_decode_tone_keeps_energy() {
        let mut enc = Encoder::new(128_000).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut tone = Tone::new(440.0, 0.5);
        let mut out_peak = 0.0f32;
        let mut sizes = vec![];
        for _ in 0..50 {
            let f = tone.next_frame();
            let p = enc.encode(&f).unwrap();
            sizes.push(p.len());
            out_peak = out_peak.max(crate::peak(&dec.decode(&p).unwrap()));
        }
        assert!(out_peak > 0.3, "decoded peak {out_peak}");
        let avg = sizes.iter().sum::<usize>() / sizes.len();
        assert!(avg < 400, "10 ms @128 kb/s should be ~160 B, got {avg}");
        assert_eq!(dec.conceal().len(), FRAME_LEN);
    }
}
