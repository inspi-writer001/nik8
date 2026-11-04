//! WebTransport client for the `sender` ingress: one persistent connection, one
//! unidirectional stream per signed transaction. `sender` validates it and forwards
//! it straight to the upcoming leaders' TPUs.

use std::{fmt, sync::Arc, time::Duration};

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use wtransport::{
    ClientConfig, Connection, Endpoint,
    endpoint::endpoint_side::Client,
    tls::{
        client::build_default_tls_config,
        rustls::{
            self, DigitallySignedStruct, RootCertStore, SignatureScheme,
            client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
            crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature},
            pki_types::{CertificateDer, ServerName, UnixTime},
        },
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Ingress {
    url: String,
    link: Mutex<Link>,
}

/// The endpoint binds a UDP socket, which needs a runtime, so it's created on first
/// use and then reused for every reconnect.
struct Link {
    config: Option<ClientConfig>,
    endpoint: Option<Endpoint<Client>>,
    connection: Option<Connection>,
}

impl Ingress {
    /// `cert_sha256`: SHA-256 of the ingress certificate (DER). The connection is
    /// accepted only if the server presents exactly that certificate.
    pub fn new(url: impl Into<String>, cert_sha256: [u8; 32]) -> Result<Self, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = build_default_tls_config(
            Arc::new(RootCertStore::empty()),
            Some(Arc::new(PinnedCertificate {
                sha256: cert_sha256,
                provider,
            })),
        );
        let config = ClientConfig::builder()
            .with_bind_default()
            .with_custom_tls(tls)
            .max_idle_timeout(Some(IDLE_TIMEOUT))
            .map_err(|error| format!("ingress idle timeout: {error}"))?
            .keep_alive_interval(Some(KEEP_ALIVE))
            .build();

        Ok(Self {
            url: url.into(),
            link: Mutex::new(Link {
                config: Some(config),
                endpoint: None,
                connection: None,
            }),
        })
    }

    /// Connects now so the first transaction doesn't pay for the handshake.
    pub async fn warm_up(&self) -> Result<(), String> {
        self.connection().await.map(|_| ())
    }

    /// Sends one signed wire transaction. Reconnects once if the connection dropped.
    pub async fn send(&self, transaction: &[u8]) -> Result<(), String> {
        let connection = self.connection().await?;
        match send_on(&connection, transaction).await {
            Ok(()) => Ok(()),
            Err(_) => {
                self.link.lock().await.connection = None;
                let connection = self.connection().await?;
                send_on(&connection, transaction).await
            }
        }
    }

    async fn connection(&self) -> Result<Connection, String> {
        let mut link = self.link.lock().await;
        if let Some(connection) = &link.connection
            && connection.quic_connection().close_reason().is_none()
        {
            return Ok(connection.clone());
        }

        if link.endpoint.is_none() {
            let config = link.config.take().expect("config is kept until the endpoint exists");
            link.endpoint =
                Some(Endpoint::client(config).map_err(|error| format!("ingress socket: {error}"))?);
        }
        let endpoint = link.endpoint.as_ref().expect("set above");
        let connection = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(&self.url))
            .await
            .map_err(|_| format!("ingress connect to {} timed out", self.url))?
            .map_err(|error| format!("ingress connect to {}: {error}", self.url))?;
        link.connection = Some(connection.clone());
        Ok(connection)
    }
}

async fn send_on(connection: &Connection, transaction: &[u8]) -> Result<(), String> {
    let mut stream = connection
        .open_uni()
        .await
        .map_err(|error| format!("ingress stream: {error}"))?
        .await
        .map_err(|error| format!("ingress stream: {error}"))?;
    stream
        .write_all(transaction)
        .await
        .map_err(|error| format!("ingress write: {error}"))?;
    stream
        .finish()
        .await
        .map_err(|error| format!("ingress finish: {error}"))
}

/// Accepts only the server certificate whose SHA-256 matches. Unlike WebTransport's
/// built-in hash pinning there's no 14-day validity limit, so a self-signed ingress
/// certificate can outlive an installed APK. Handshake signatures are still verified.
struct PinnedCertificate {
    sha256: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl fmt::Debug for PinnedCertificate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PinnedCertificate")
    }
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if Sha256::digest(end_entity.as_ref()).as_slice() == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("ingress certificate does not match the pinned hash".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Parses a hex SHA-256 (64 chars, `:` separators allowed, as openssl prints them).
pub fn parse_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    let digits: Vec<u8> = hex.bytes().filter(|byte| *byte != b':').collect();
    if digits.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, pair) in digits.chunks(2).enumerate() {
        out[index] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::parse_sha256_hex;

    #[test]
    fn parses_hex_digests() {
        let hex = "fc1bff72753b05d9de0db8ffa58384adc19d07f515f0025cf9018976fa3e8873";
        let digest = parse_sha256_hex(hex).unwrap();
        assert_eq!(digest[0], 0xfc);
        assert_eq!(digest[31], 0x73);
        assert_eq!(parse_sha256_hex(&hex.to_uppercase()), Some(digest));
        assert_eq!(parse_sha256_hex("FC:1B"), None);
        assert_eq!(parse_sha256_hex(&"zz".repeat(32)), None);
    }
}
