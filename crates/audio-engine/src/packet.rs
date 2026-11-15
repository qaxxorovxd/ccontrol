//! Authenticated media / report packets.
//!
//! Layout (big-endian):
//! ```text
//! 0  magic "CCA1"           4
//! 4  kind (0=media,1=report)1
//! 5  flags                  1
//! 6  reserved               2
//! 8  route tag              4   (first 4 bytes of SHA-256(route_id))
//! 12 seq                    8
//! 20 send time (µs, sender clock) 8
//! 28 ciphertext || 16-byte Poly1305 tag
//! ```
//! The 28-byte header is the AEAD associated data. Nonce = kind || 000 || seq,
//! unique per (key, direction) because each direction has its own `kind` and
//! a strictly increasing sequence. The key is 32 random bytes chosen by the
//! controller and delivered to both agents over their mutually-authenticated
//! TLS control channels. Primitive: ChaCha20-Poly1305 from `ring`.

use crate::AudioError;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};

pub const MAGIC: &[u8; 4] = b"CCA1";
pub const HEADER_LEN: usize = 28;
pub const TAG_LEN: usize = 16;
pub const MAX_PACKET: usize = 1400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Media = 0,
    Report = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub route_tag: u32,
    pub seq: u64,
    pub send_time_us: u64,
}

pub struct StreamKey {
    key: LessSafeKey,
    route_tag: u32,
}

pub fn route_tag(route_id: &str) -> u32 {
    let d = ring::digest::digest(&ring::digest::SHA256, route_id.as_bytes());
    let b = d.as_ref();
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl StreamKey {
    pub fn new(route_id: &str, key_hex: &str) -> Result<Self, AudioError> {
        let raw = decode_hex(key_hex)
            .ok_or_else(|| AudioError::Invalid("stream key must be 64 hex chars".into()))?;
        let key = UnboundKey::new(&CHACHA20_POLY1305, &raw)
            .map_err(|_| AudioError::Invalid("bad key".into()))?;
        Ok(Self {
            key: LessSafeKey::new(key),
            route_tag: route_tag(route_id),
        })
    }

    pub fn route_tag(&self) -> u32 {
        self.route_tag
    }

    fn nonce(kind: Kind, seq: u64) -> Nonce {
        let mut n = [0u8; 12];
        n[0] = kind as u8;
        n[4..].copy_from_slice(&seq.to_be_bytes());
        Nonce::assume_unique_for_key(n)
    }

    pub fn seal(
        &self,
        kind: Kind,
        seq: u64,
        send_time_us: u64,
        payload: &[u8],
    ) -> Result<Vec<u8>, AudioError> {
        if HEADER_LEN + payload.len() + TAG_LEN > MAX_PACKET {
            return Err(AudioError::Invalid("payload too large".into()));
        }
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len() + TAG_LEN);
        out.extend_from_slice(MAGIC);
        out.push(kind as u8);
        out.push(0);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&self.route_tag.to_be_bytes());
        out.extend_from_slice(&seq.to_be_bytes());
        out.extend_from_slice(&send_time_us.to_be_bytes());
        let mut body = payload.to_vec();
        self.key
            .seal_in_place_append_tag(
                Self::nonce(kind, seq),
                Aad::from(&out[..HEADER_LEN]),
                &mut body,
            )
            .map_err(|_| AudioError::Net("seal failed".into()))?;
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Verify and decrypt. Returns `None` for anything not authentic.
    pub fn open(&self, packet: &[u8]) -> Option<(Header, Vec<u8>)> {
        if packet.len() < HEADER_LEN + TAG_LEN || packet.len() > MAX_PACKET || &packet[..4] != MAGIC
        {
            return None;
        }
        let kind = match packet[4] {
            0 => Kind::Media,
            1 => Kind::Report,
            _ => return None,
        };
        let tag = u32::from_be_bytes(packet[8..12].try_into().ok()?);
        if tag != self.route_tag {
            return None;
        }
        let seq = u64::from_be_bytes(packet[12..20].try_into().ok()?);
        let send_time_us = u64::from_be_bytes(packet[20..28].try_into().ok()?);
        let mut body = packet[HEADER_LEN..].to_vec();
        let plain = self
            .key
            .open_in_place(
                Self::nonce(kind, seq),
                Aad::from(&packet[..HEADER_LEN]),
                &mut body,
            )
            .ok()?;
        let len = plain.len();
        body.truncate(len);
        Some((
            Header {
                kind,
                route_tag: tag,
                seq,
                send_time_us,
            },
            body,
        ))
    }
}

pub fn decode_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

pub fn random_key_hex() -> String {
    use rand::RngCore;
    let mut k = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut k);
    k.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sliding-window replay protection (64 packets) per direction.
#[derive(Debug, Default)]
pub struct ReplayWindow {
    highest: Option<u64>,
    bitmap: u64,
}

impl ReplayWindow {
    /// Returns false for duplicates and packets older than the window.
    pub fn accept(&mut self, seq: u64) -> bool {
        match self.highest {
            None => {
                self.highest = Some(seq);
                self.bitmap = 1;
                true
            }
            Some(h) if seq > h => {
                let shift = seq - h;
                self.bitmap = if shift >= 64 {
                    1
                } else {
                    (self.bitmap << shift) | 1
                };
                self.highest = Some(seq);
                true
            }
            Some(h) => {
                let off = h - seq;
                if off >= 64 || self.bitmap & (1 << off) != 0 {
                    return false;
                }
                self.bitmap |= 1 << off;
                true
            }
        }
    }
}

/// Receiver report payload (dest -> source), 48 bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Report {
    pub frames_played: u64,
    pub packets: u64,
    pub lost: u64,
    pub late: u64,
    /// Send time of the newest media packet received (sender clock, µs).
    pub echo_send_time_us: u64,
    /// Time the receiver held that packet before sending this report (µs).
    pub hold_us: u32,
    pub jitter_us: u32,
    pub buffer_us: u32,
    pub peak_milli: u16,
    pub underruns: u16,
}

impl Report {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(52);
        for x in [
            self.frames_played,
            self.packets,
            self.lost,
            self.late,
            self.echo_send_time_us,
        ] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        for x in [self.hold_us, self.jitter_us, self.buffer_us] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v.extend_from_slice(&self.peak_milli.to_be_bytes());
        v.extend_from_slice(&self.underruns.to_be_bytes());
        v
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 56 {
            return None;
        }
        let u64_at = |i: usize| u64::from_be_bytes(b[i..i + 8].try_into().unwrap());
        let u32_at = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_be_bytes(b[i..i + 2].try_into().unwrap());
        Some(Self {
            frames_played: u64_at(0),
            packets: u64_at(8),
            lost: u64_at(16),
            late: u64_at(24),
            echo_send_time_us: u64_at(32),
            hold_us: u32_at(40),
            jitter_us: u32_at(44),
            buffer_us: u32_at(48),
            peak_milli: u16_at(52),
            underruns: u16_at(54),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip() {
        let k = random_key_hex();
        let a = StreamKey::new("r1", &k).unwrap();
        let p = a.seal(Kind::Media, 5, 123, b"hello").unwrap();
        let (h, body) = a.open(&p).unwrap();
        assert_eq!(body, b"hello");
        assert_eq!(h.seq, 5);
        assert_eq!(h.send_time_us, 123);
    }

    #[test]
    fn rejects_tampering_wrong_key_and_wrong_route() {
        let k = random_key_hex();
        let a = StreamKey::new("r1", &k).unwrap();
        let mut p = a.seal(Kind::Media, 1, 0, b"abc").unwrap();
        // flip a header bit (AAD) -> fail
        let mut q = p.clone();
        q[21] ^= 1;
        assert!(a.open(&q).is_none());
        // flip a ciphertext bit -> fail
        let n = p.len();
        p[n - 1] ^= 1;
        assert!(a.open(&p).is_none());
        let p = a.seal(Kind::Media, 1, 0, b"abc").unwrap();
        assert!(StreamKey::new("r1", &random_key_hex())
            .unwrap()
            .open(&p)
            .is_none());
        assert!(StreamKey::new("r2", &k).unwrap().open(&p).is_none());
        // kind is bound into the nonce: relabelled media->report fails
        let mut q = p.clone();
        q[4] = 1;
        assert!(a.open(&q).is_none());
    }

    #[test]
    fn replay_window() {
        let mut w = ReplayWindow::default();
        assert!(w.accept(10));
        assert!(!w.accept(10));
        assert!(w.accept(12));
        assert!(w.accept(11));
        assert!(!w.accept(11));
        assert!(w.accept(100));
        assert!(!w.accept(30)); // older than window
        assert!(w.accept(99));
    }

    #[test]
    fn report_round_trip() {
        let r = Report {
            frames_played: 9,
            packets: 8,
            lost: 1,
            late: 2,
            echo_send_time_us: 77,
            hold_us: 3,
            jitter_us: 4,
            buffer_us: 5,
            peak_milli: 600,
            underruns: 1,
        };
        assert_eq!(Report::decode(&r.encode()), Some(r));
    }

    #[test]
    fn hex() {
        assert!(decode_hex(&"0f".repeat(32)).is_some());
        assert!(decode_hex("0f").is_none());
        assert!(decode_hex(&"zz".repeat(32)).is_none());
    }
}
