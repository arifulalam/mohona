//! An HTTPS version of the test server, with certificates made on the spot.

use std::io;
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, IsCa, KeyPair};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

use crate::Config;

/// A certificate authority and a server certificate it signed.
#[derive(Clone)]
pub struct Certificates {
    /// The authority, as PEM. Give this to a client that should trust the server.
    pub ca_pem: String,
    leaf_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl Certificates {
    /// A second handle to the same certificates (to hand to a server task).
    pub fn clone_for_server(other: &Certificates) -> Certificates {
        Certificates {
            ca_pem: other.ca_pem.clone(),
            leaf_der: other.leaf_der.clone(),
            key_der: other.key_der.clone(),
        }
    }

    /// A fresh authority and a certificate for `localhost` and `127.0.0.1`,
    /// signed by it. With `expired`, the certificate ended long ago.
    pub fn new(expired: bool) -> Self {
        let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Mohona test authority");
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();

        let mut params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
        if expired {
            params.not_before = rcgen::date_time_ymd(2020, 1, 1);
            params.not_after = rcgen::date_time_ymd(2020, 1, 2);
        }
        let key = KeyPair::generate().unwrap();
        let leaf = params
            .signed_by(&key, &rcgen::Issuer::from_params(&ca_params, &ca_key))
            .unwrap();
        Self {
            ca_pem: ca.pem(),
            leaf_der: leaf.der().to_vec(),
            key_der: key.serialize_der(),
        }
    }

    /// A TLS acceptor presenting this certificate.
    pub fn acceptor(&self) -> TlsAcceptor {
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.leaf_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone())),
            )
            .unwrap();
        TlsAcceptor::from(Arc::new(config))
    }
}

/// Like `serve`, but over HTTPS using `certificates`.
pub async fn serve_tls(
    listener: TcpListener,
    config: Config,
    certificates: &Certificates,
) -> io::Result<()> {
    crate::serve_with(listener, config, Some(certificates.acceptor())).await
}
