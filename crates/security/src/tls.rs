//! TLS 1.3 configurations with application-layer fingerprint pinning.
//!
//! The handshake still proves possession of the private key behind each
//! certificate (CertificateVerify signatures are fully checked). What is
//! skipped is CA chain validation, which is meaningless for self-signed
//! device identities; instead the peer's SPKI fingerprint is checked against
//! the trust store (server side) or the pinned/QR fingerprint (client side).

use crate::{fingerprint_of_cert, DeviceIdentity, SecurityError};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};
use std::sync::Arc;

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Server config: TLS 1.3 only, client certificate mandatory.
pub fn server_config(identity: &DeviceIdentity) -> Result<Arc<ServerConfig>, SecurityError> {
    let provider = provider();
    let cfg = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(AnyClientCert { provider }))
        .with_single_cert(vec![identity.cert()], identity.key_der())?;
    Ok(Arc::new(cfg))
}

/// Client config (desktop-to-desktop and tests). If `expected_server_fp` is
/// set the handshake fails unless the server's SPKI fingerprint matches.
pub fn client_config(
    identity: &DeviceIdentity,
    expected_server_fp: Option<String>,
) -> Result<Arc<ClientConfig>, SecurityError> {
    let provider = provider();
    let cfg = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServer {
            provider,
            expected_server_fp,
        }))
        .with_client_auth_cert(vec![identity.cert()], identity.key_der())?;
    Ok(Arc::new(cfg))
}

/// Fingerprint of the first certificate a peer presented.
pub fn peer_fingerprint(certs: Option<&[CertificateDer<'_>]>) -> Result<String, SecurityError> {
    let first = certs
        .and_then(|c| c.first())
        .ok_or_else(|| SecurityError::Invalid("peer presented no certificate".into()))?;
    fingerprint_of_cert(first.as_ref())
}

#[derive(Debug)]
struct AnyClientCert {
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for AnyClientCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // Authorization happens after the handshake against the trust store.
        // Here we only require a parseable certificate.
        fingerprint_of_cert(end_entity.as_ref()).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct PinnedServer {
    provider: Arc<CryptoProvider>,
    expected_server_fp: Option<String>,
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint_of_cert(end_entity.as_ref()).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        match &self.expected_server_fp {
            Some(expected) if !expected.eq_ignore_ascii_case(&fp) => {
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::ApplicationVerificationFailure,
                ))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn handshake(
        server: &DeviceIdentity,
        client: &DeviceIdentity,
        pin: Option<String>,
    ) -> Result<(String, String), String> {
        let acceptor = tokio_rustls::TlsAcceptor::from(server_config(server).unwrap());
        let connector = tokio_rustls::TlsConnector::from(client_config(client, pin).unwrap());
        let (a, b) = tokio::io::duplex(16 * 1024);
        let srv = tokio::spawn(async move {
            let mut s = acceptor.accept(a).await.map_err(|e| e.to_string())?;
            let fp = peer_fingerprint(s.get_ref().1.peer_certificates()).unwrap();
            let mut buf = [0u8; 2];
            s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            Ok::<_, String>(fp)
        });
        let name = ServerName::try_from("ccontrol.invalid").unwrap();
        let mut c = connector
            .connect(name, b)
            .await
            .map_err(|e| e.to_string())?;
        let server_fp = peer_fingerprint(c.get_ref().1.peer_certificates()).unwrap();
        c.write_all(b"ok").await.map_err(|e| e.to_string())?;
        let client_fp = srv.await.unwrap()?;
        Ok((server_fp, client_fp))
    }

    #[tokio::test]
    async fn mutual_tls_exposes_both_fingerprints() {
        let s = DeviceIdentity::generate("srv").unwrap();
        let c = DeviceIdentity::generate("cli").unwrap();
        let (sfp, cfp) = handshake(&s, &c, None).await.unwrap();
        assert_eq!(sfp, s.fingerprint);
        assert_eq!(cfp, c.fingerprint);
    }

    #[tokio::test]
    async fn pinned_fingerprint_must_match() {
        let s = DeviceIdentity::generate("srv").unwrap();
        let c = DeviceIdentity::generate("cli").unwrap();
        assert!(handshake(&s, &c, Some(s.fingerprint.clone())).await.is_ok());
        let impostor = DeviceIdentity::generate("evil").unwrap();
        assert!(handshake(&impostor, &c, Some(s.fingerprint.clone()))
            .await
            .is_err());
    }
}
