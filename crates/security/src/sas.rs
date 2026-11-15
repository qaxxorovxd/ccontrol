//! Commit/reveal short authentication string (SAS) for pairing.
//!
//! The TLS channel already binds both parties' certificates. A
//! man-in-the-middle must present its *own* certificates to each side, so the
//! fingerprints mixed into the SAS differ on the two screens. The commit/reveal
//! order (client commits to its nonce before seeing the server nonce; the
//! server picks its nonce before seeing the client nonce) prevents an attacker
//! from grinding nonces until the two 6-digit codes collide: its success
//! probability per attempt is 1e-6, and attempts are rate-limited and each
//! needs the owner to accept a request on the desktop.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const NONCE_LEN: usize = 32;

pub fn random_nonce() -> [u8; NONCE_LEN] {
    use rand::RngCore;
    let mut n = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut n);
    n
}

pub fn commitment(nonce: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"ccontrol-commit-v1");
    h.update(nonce);
    hex::encode(h.finalize())
}

/// Constant-time check that `nonce` opens `commitment_hex`.
pub fn verify_commitment(commitment_hex: &str, nonce: &[u8]) -> bool {
    let expected = commitment(nonce);
    expected.len() == commitment_hex.len()
        && bool::from(expected.as_bytes().ct_eq(commitment_hex.as_bytes()))
}

/// Six-digit code shown on both screens.
pub fn compute_sas(
    server_fp_hex: &str,
    client_fp_hex: &str,
    client_nonce: &[u8],
    server_nonce: &[u8],
) -> String {
    let mut h = Sha256::new();
    h.update(b"ccontrol-sas-v1");
    h.update(server_fp_hex.as_bytes());
    h.update(client_fp_hex.as_bytes());
    h.update(client_nonce);
    h.update(server_nonce);
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{n:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_verifies() {
        let n = random_nonce();
        let c = commitment(&n);
        assert!(verify_commitment(&c, &n));
        let mut other = n;
        other[0] ^= 1;
        assert!(!verify_commitment(&c, &other));
    }

    #[test]
    fn sas_is_six_digits_and_binds_inputs() {
        let s = compute_sas("aa", "bb", &[1; 32], &[2; 32]);
        assert_eq!(s.len(), 6);
        assert!(s.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(s, compute_sas("aa", "bc", &[1; 32], &[2; 32]));
        assert_ne!(s, compute_sas("ab", "bb", &[1; 32], &[2; 32]));
    }

    /// Golden vector shared with the Android implementation
    /// (android/core/src/test/.../SasTest.kt).
    #[test]
    fn golden_vector() {
        let s = compute_sas(
            "1111111111111111111111111111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222222222222222222222222222",
            &[0xAB; 32],
            &[0xCD; 32],
        );
        assert_eq!(s, GOLDEN_SAS);
        assert_eq!(commitment(&[0xAB; 32]), GOLDEN_COMMIT);
    }

    // Cross-checked against an independent Python hashlib computation.
    const GOLDEN_SAS: &str = "285326";
    const GOLDEN_COMMIT: &str = "904fd8d0fbe2bc606afa210ab9b1dcf33974f2e7551da819d9a91318a237132f";
}
