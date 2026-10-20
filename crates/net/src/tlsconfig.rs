//! TLS settings for connections Mohona makes itself (FTPS), following the same
//! rules as HTTPS: the system's trusted authorities, plus an optional extra
//! authority, or no checking at all when asked.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use rustls_platform_verifier::Verifier;

use crate::{NetError, TlsOptions};

/// Accepts every certificate. Only for `--insecure`.
#[derive(Debug)]
struct AcceptAll(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for AcceptAll {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

/// A client configuration for these options. Connections made from the same
/// configuration can resume each other's TLS sessions, which some FTPS
/// servers demand of the data connection.
pub(crate) fn client_config(tls: Option<&TlsOptions>) -> Result<Arc<ClientConfig>, NetError> {
    let provider: Arc<CryptoProvider> = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| NetError::Tls(e.to_string()))?;
    let options = tls.cloned().unwrap_or_default();
    let config = if options.insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAll(
                provider.signature_verification_algorithms,
            )))
            .with_no_client_auth()
    } else {
        let mut extra = Vec::new();
        if let Some(path) = &options.ca_file {
            let bad = |reason: String| NetError::InvalidCaFile {
                path: path.clone(),
                reason,
            };
            let pem = std::fs::read(path).map_err(|e| bad(e.to_string()))?;
            for certificate in CertificateDer::pem_slice_iter(&pem) {
                extra.push(certificate.map_err(|e| bad(e.to_string()))?);
            }
            if extra.is_empty() {
                return Err(bad("it holds no certificate".into()));
            }
        }
        let verifier = Verifier::new_with_extra_roots(extra, provider)
            .map_err(|e| NetError::Tls(e.to_string()))?;
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth()
    };
    Ok(Arc::new(config))
}
