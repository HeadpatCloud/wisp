use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};

// Accepts any chain and name: trust comes from the pinned fingerprint, checked after the handshake.
#[derive(Debug)]
struct PinnedLater(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for PinnedLater {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

// rustls has only a Debug dump for a certificate it cannot read.
pub fn failure(e: &std::io::Error) -> String {
    let unsupported = match e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        // An RSA key rustls finds too small is reported exactly as a forged signature is.
        Some(rustls::Error::InvalidCertificate(CertificateError::BadSignature)) => {
            return "the server's certificate could not be verified \
                    (bad signature, or an RSA key under 2048 bits)"
                .into();
        }
        Some(rustls::Error::InvalidCertificate(CertificateError::Other(other)))
            if other.to_string() == "UnsupportedCertVersion" =>
        {
            Some("not X.509 version 3")
        }
        Some(rustls::Error::InvalidCertificate(
            CertificateError::UnsupportedSignatureAlgorithmContext { .. }
            | CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. },
        )) => Some("signature algorithm"),
        _ => None,
    };
    match unsupported {
        Some(reason) => format!("the server's certificate is not supported ({reason})"),
        None => format!("TLS handshake failed: {e}"),
    }
}

// The ring provider is named because test builds link aws-lc-rs too, and rustls panics there when
// it has to pick one.
pub fn client_config() -> ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinnedLater(provider.signature_verification_algorithms);
    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("the ring provider supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use rustls::CertificateError;

    use super::*;

    fn failed(e: CertificateError) -> String {
        failure(&Error::new(ErrorKind::InvalidData, rustls::Error::from(e)))
    }

    #[test]
    fn certificates_rustls_cannot_use_are_named_in_words() {
        let algorithm = CertificateError::UnsupportedSignatureAlgorithmContext {
            signature_algorithm_id: vec![],
            supported_algorithms: vec![],
        };
        assert_eq!(
            failed(algorithm),
            "the server's certificate is not supported (signature algorithm)"
        );
        assert_eq!(
            failed(CertificateError::BadSignature),
            "the server's certificate could not be verified \
             (bad signature, or an RSA key under 2048 bits)"
        );
    }

    #[test]
    fn any_other_failure_keeps_the_generic_text() {
        assert_eq!(
            failed(CertificateError::Expired),
            "TLS handshake failed: invalid peer certificate: Expired"
        );
        let plain = Error::new(ErrorKind::UnexpectedEof, "eof");
        assert_eq!(failure(&plain), "TLS handshake failed: eof");
    }
}
