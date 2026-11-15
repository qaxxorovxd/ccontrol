use crate::{protect, SecurityError};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};
use sha2::{Digest, Sha256};
use std::path::Path;

const CERT_FILE: &str = "identity.cert.der";
const KEY_FILE: &str = "identity.key.protected";

/// Long-lived device identity: P-256 key + self-signed certificate.
#[derive(Clone)]
pub struct DeviceIdentity {
    pub cert_der: Vec<u8>,
    key_pkcs8_der: Vec<u8>,
    pub fingerprint: String,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("fingerprint", &self.fingerprint)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl DeviceIdentity {
    /// Generate a fresh identity (not persisted).
    pub fn generate(common_name: &str) -> Result<Self, SecurityError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(vec!["ccontrol.invalid".to_owned()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        dn.push(DnType::OrganizationName, "ccontrol device");
        params.distinguished_name = dn;
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2124, 1, 1);
        let cert = params.self_signed(&key)?;
        let cert_der = cert.der().to_vec();
        let fingerprint = fingerprint_of_cert(&cert_der)?;
        Ok(Self {
            cert_der,
            key_pkcs8_der: key.serialize_der(),
            fingerprint,
        })
    }

    /// Load the identity from `dir`, creating and persisting one on first run.
    pub fn load_or_create(dir: &Path, common_name: &str) -> Result<Self, SecurityError> {
        std::fs::create_dir_all(dir)?;
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);
        if cert_path.exists() && key_path.exists() {
            let cert_der = std::fs::read(&cert_path)?;
            let key_pkcs8_der = protect::unprotect(&std::fs::read(&key_path)?)?;
            // Make sure key and certificate still belong together.
            let kp = KeyPair::try_from(key_pkcs8_der.as_slice())
                .map_err(|e| SecurityError::Invalid(format!("stored key unreadable: {e}")))?;
            let fingerprint = fingerprint_of_cert(&cert_der)?;
            let key_fp = hex::encode(Sha256::digest(kp.public_key_der()));
            if key_fp != fingerprint {
                return Err(SecurityError::Invalid(
                    "stored key does not match stored certificate".into(),
                ));
            }
            return Ok(Self {
                cert_der,
                key_pkcs8_der,
                fingerprint,
            });
        }
        let id = Self::generate(common_name)?;
        protect::write_private(&key_path, &protect::protect(&id.key_pkcs8_der)?)?;
        write_atomic(&cert_path, &id.cert_der)?;
        Ok(id)
    }

    pub fn key_der(&self) -> rustls::pki_types::PrivateKeyDer<'static> {
        rustls::pki_types::PrivateKeyDer::Pkcs8(self.key_pkcs8_der.clone().into())
    }

    pub fn cert(&self) -> rustls::pki_types::CertificateDer<'static> {
        rustls::pki_types::CertificateDer::from(self.cert_der.clone())
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

/// `hex(SHA-256(SubjectPublicKeyInfo DER))` of an X.509 certificate.
pub fn fingerprint_of_cert(cert_der: &[u8]) -> Result<String, SecurityError> {
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der)
        .map_err(|e| SecurityError::CertParse(e.to_string()))?;
    Ok(hex::encode(Sha256::digest(
        cert.tbs_certificate.subject_pki.raw,
    )))
}

/// Human-friendly short form, e.g. `3F2A-91C0-77DE-0B14`.
pub fn short_fingerprint(fp: &str) -> String {
    fp.chars()
        .take(16)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>().to_uppercase())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let a = DeviceIdentity::load_or_create(dir.path(), "test").unwrap();
        let b = DeviceIdentity::load_or_create(dir.path(), "test").unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 64);
    }

    #[test]
    fn distinct_identities_differ() {
        let a = DeviceIdentity::generate("a").unwrap();
        let b = DeviceIdentity::generate("b").unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn tampered_cert_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        DeviceIdentity::load_or_create(dir.path(), "x").unwrap();
        let other = DeviceIdentity::generate("y").unwrap();
        std::fs::write(dir.path().join(CERT_FILE), &other.cert_der).unwrap();
        assert!(DeviceIdentity::load_or_create(dir.path(), "x").is_err());
    }

    #[test]
    fn short_form() {
        assert_eq!(
            short_fingerprint("0123456789abcdef0011"),
            "0123-4567-89AB-CDEF"
        );
    }
}
