//! Security primitives for ccontrol.
//!
//! * [`identity`]: stable per-device ECDSA P-256 identity with a self-signed
//!   certificate. The device ID is `hex(SHA-256(SubjectPublicKeyInfo))`, so it
//!   is bound to the key — never to an IP address, hostname or MAC.
//! * [`tls`]: rustls TLS 1.3 configurations. Certificates are *not* validated
//!   against a CA; trust is established by pairing and enforced by pinning the
//!   SPKI fingerprint at the application layer.
//! * [`sas`]: commit/reveal short-authentication-string used during pairing.
//! * [`protect`]: at-rest protection of the private key (DPAPI on Windows,
//!   0600 file permissions on Unix).
//!
//! No custom cryptography is implemented here; all primitives come from
//! rustls/ring and `sha2`.

pub mod identity;
pub mod protect;
pub mod sas;
pub mod tls;

pub use identity::{fingerprint_of_cert, short_fingerprint, DeviceIdentity};

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("certificate generation failed: {0}")]
    CertGen(#[from] rcgen::Error),
    #[error("certificate parse failed: {0}")]
    CertParse(String),
    #[error("tls configuration: {0}")]
    Tls(#[from] rustls::Error),
    #[error("key protection: {0}")]
    Protect(String),
    #[error("invalid input: {0}")]
    Invalid(String),
}
