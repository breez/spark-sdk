//! `PostgreSQL` connection pool creation and TLS configuration.

use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::Pool;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject};
use rustls::server::ParsedCertificate;
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
};
use tokio_postgres::Config as PgConfig;
use tokio_postgres_rustls::MakeRustlsConnect;
use webpki_roots::TLS_SERVER_ROOTS;

use crate::config::PostgresStorageConfig;
use crate::error::PostgresError;

/// Certificate verifier that accepts any server certificate.
/// This is used for `sslmode=require` which only ensures encryption,
/// not server identity verification.
#[derive(Debug)]
struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

/// Certificate verifier that validates the certificate chain against trusted roots
/// but does not verify the server hostname. This is used for `sslmode=verify-ca`.
#[derive(Debug)]
struct CaOnlyVerifier {
    roots: Arc<RootCertStore>,
}

impl CaOnlyVerifier {
    fn new(roots: RootCertStore) -> Self {
        Self {
            roots: Arc::new(roots),
        }
    }
}

impl ServerCertVerifier for CaOnlyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let cert = ParsedCertificate::try_from(end_entity)?;

        // Build the certificate chain for verification
        let mut chain = vec![end_entity.clone()];
        chain.extend(intermediates.iter().cloned());

        // Verify the certificate chain against the root store
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            default_provider().signature_verification_algorithms.all,
        )?;

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Parses PEM-encoded certificates and returns a `RootCertStore` containing them.
pub fn parse_pem_to_root_store(pem: &str) -> Result<RootCertStore, PostgresError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            PostgresError::Initialization(format!("Failed to parse PEM certificates: {e}"))
        })?;

    if certs.is_empty() {
        return Err(PostgresError::Initialization(
            "No valid certificates found in PEM data".to_string(),
        ));
    }

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store.add(cert).map_err(|e| {
            PostgresError::Initialization(format!("Failed to add certificate to store: {e}"))
        })?;
    }

    Ok(root_store)
}

/// Creates a rustls `ClientConfig` that verifies the server certificate chain.
///
/// # Arguments
/// * `verify_hostname` - If true, also verifies the server hostname matches the certificate (verify-full).
///   If false, only verifies the certificate chain (verify-ca).
/// * `custom_ca` - Optional PEM-encoded CA certificate(s). If None, uses Mozilla's root store.
pub fn make_tls_config_verifying(
    verify_hostname: bool,
    custom_ca: Option<&str>,
) -> Result<ClientConfig, PostgresError> {
    let root_store = if let Some(pem) = custom_ca {
        parse_pem_to_root_store(pem)?
    } else {
        let mut root_store = RootCertStore::empty();
        root_store.extend(TLS_SERVER_ROOTS.iter().cloned());
        root_store
    };

    let config = if verify_hostname {
        // verify-full: use the standard WebPKI verifier which checks hostname
        ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth()
    } else {
        // verify-ca: use our custom verifier that only checks the certificate chain
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(CaOnlyVerifier::new(root_store)))
            .with_no_client_auth()
    };

    Ok(config)
}

/// Creates a rustls `ClientConfig` that accepts any server certificate.
/// This is appropriate for `sslmode=require` which ensures encrypted connections
/// but does not verify the server's identity.
fn make_tls_config() -> ClientConfig {
    ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth()
}

/// Internal representation of SSL modes, including verify-ca and verify-full
/// that are not exposed by tokio-postgres's `SslMode` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SslModeExt {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

/// Extracts the sslmode from a connection string.
/// Handles both key-value format and URI format.
fn parse_sslmode_from_connection_string(conn_str: &str) -> SslModeExt {
    /// Parses an sslmode value string into an `SslModeExt`.
    fn parse_sslmode_value(value: &str) -> SslModeExt {
        match value {
            "disable" => SslModeExt::Disable,
            "require" => SslModeExt::Require,
            "verify-ca" => SslModeExt::VerifyCa,
            "verify-full" => SslModeExt::VerifyFull,
            // "prefer" and unknown values default to Prefer
            _ => SslModeExt::Prefer,
        }
    }

    // Check for URI format: postgres://...?sslmode=...
    if conn_str.starts_with("postgres://") || conn_str.starts_with("postgresql://") {
        if let Some(query) = conn_str.split_once('?').map(|(_, q)| q) {
            for param in query.split('&') {
                if let Some(("sslmode", value)) = param.split_once('=') {
                    return parse_sslmode_value(value);
                }
            }
        }
    } else {
        // Key-value format: host=... sslmode=...
        for part in conn_str.split_whitespace() {
            if let Some(("sslmode", value)) = part.split_once('=') {
                return parse_sslmode_value(value);
            }
        }
    }

    // Default to Prefer if not specified
    SslModeExt::Prefer
}

/// Applies pool configuration options from `PostgresStorageConfig` to a deadpool-postgres config.
fn apply_pool_config(config: &PostgresStorageConfig) -> deadpool_postgres::PoolConfig {
    deadpool_postgres::PoolConfig {
        max_size: config.max_pool_size as usize,
        timeouts: deadpool::managed::Timeouts {
            wait: config.wait_timeout_secs.map(Duration::from_secs),
            create: config.create_timeout_secs.map(Duration::from_secs),
            recycle: config.recycle_timeout_secs.map(Duration::from_secs),
        },
        queue_mode: config.queue_mode.into(),
    }
}

/// Creates a `PostgreSQL` connection pool from the given configuration.
pub fn create_pool(config: &PostgresStorageConfig) -> Result<Pool, PostgresError> {
    let pg_config: PgConfig = config
        .connection_string
        .parse()
        .map_err(|e| PostgresError::Initialization(format!("Invalid connection string: {e}")))?;

    let ssl_mode = parse_sslmode_from_connection_string(&config.connection_string);
    let pool_config = apply_pool_config(config);

    match ssl_mode {
        SslModeExt::Disable => {
            let manager = deadpool_postgres::Manager::new(pg_config, tokio_postgres::NoTls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| PostgresError::Initialization(e.to_string()))
        }
        SslModeExt::Prefer | SslModeExt::Require => {
            let tls_config = make_tls_config();
            let tls = MakeRustlsConnect::new(tls_config);
            let manager = deadpool_postgres::Manager::new(pg_config, tls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| PostgresError::Initialization(e.to_string()))
        }
        SslModeExt::VerifyCa => {
            let tls_config = make_tls_config_verifying(false, config.root_ca_pem.as_deref())?;
            let tls = MakeRustlsConnect::new(tls_config);
            let manager = deadpool_postgres::Manager::new(pg_config, tls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| PostgresError::Initialization(e.to_string()))
        }
        SslModeExt::VerifyFull => {
            let tls_config = make_tls_config_verifying(true, config.root_ca_pem.as_deref())?;
            let tls = MakeRustlsConnect::new(tls_config);
            let manager = deadpool_postgres::Manager::new(pg_config, tls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| PostgresError::Initialization(e.to_string()))
        }
    }
}

/// Maps a deadpool-postgres pool error to `PostgresError`.
/// Pool errors (exhaustion, timeout) are connection-related.
#[allow(clippy::needless_pass_by_value)]
pub fn map_pool_error(e: deadpool_postgres::PoolError) -> PostgresError {
    PostgresError::Connection(e.to_string())
}

/// Maps a tokio-postgres database error to `PostgresError`.
/// Connection-class errors (Class 08) and closed connections are mapped to `Connection`,
/// other errors are mapped to `Database`.
#[allow(clippy::needless_pass_by_value)]
pub fn map_db_error(e: tokio_postgres::Error) -> PostgresError {
    // Check if the connection is closed
    if e.is_closed() {
        return PostgresError::Connection(e.to_string());
    }
    // Check SQL state codes for connection errors (Class 08)
    if let Some(code) = e.code()
        && code.code().starts_with("08")
    {
        return PostgresError::Connection(e.to_string());
    }
    PostgresError::Database(e.to_string())
}

impl From<tokio_postgres::Error> for PostgresError {
    fn from(value: tokio_postgres::Error) -> Self {
        map_db_error(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generates a self-signed CA certificate in PEM format for testing.
    fn generate_test_ca_pem(common_name: &str) -> String {
        let mut params = rcgen::CertificateParams::new(vec![]).expect("valid params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, common_name);
        let cert = params
            .self_signed(&rcgen::KeyPair::generate().expect("valid keypair"))
            .expect("valid cert");
        cert.pem()
    }

    #[test]
    fn test_parse_valid_pem() {
        let test_ca_pem = generate_test_ca_pem("testca1");
        let result = parse_pem_to_root_store(&test_ca_pem);
        assert!(result.is_ok(), "Expected valid PEM to parse successfully");
        let store = result.unwrap();
        assert_eq!(store.len(), 1, "Expected exactly one certificate in store");
    }

    #[test]
    fn test_parse_invalid_pem() {
        let invalid_pem = "not a valid pem certificate";
        let result = parse_pem_to_root_store(invalid_pem);
        assert!(result.is_err(), "Expected invalid PEM to fail parsing");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PostgresError::Initialization(_)),
            "Expected Initialization error"
        );
    }

    #[test]
    fn test_parse_empty_pem() {
        let empty_pem = "";
        let result = parse_pem_to_root_store(empty_pem);
        assert!(result.is_err(), "Expected empty PEM to fail");
        let err = result.unwrap_err();
        match err {
            PostgresError::Initialization(msg) => {
                assert!(
                    msg.contains("No valid certificates"),
                    "Expected 'No valid certificates' error message, got: {msg}"
                );
            }
            _ => panic!("Expected Initialization error"),
        }
    }

    #[test]
    fn test_parse_multiple_certs() {
        let test_ca_pem_1 = generate_test_ca_pem("testca1");
        let test_ca_pem_2 = generate_test_ca_pem("testca2");
        let multiple_pem = format!("{test_ca_pem_1}\n{test_ca_pem_2}");
        let result = parse_pem_to_root_store(&multiple_pem);
        assert!(
            result.is_ok(),
            "Expected multiple PEM certs to parse successfully"
        );
        let store = result.unwrap();
        assert_eq!(store.len(), 2, "Expected two certificates in store");
    }

    #[test]
    fn test_tls_config_with_webpki_roots() {
        // verify-full without custom CA should use Mozilla roots
        let result = make_tls_config_verifying(true, None);
        assert!(
            result.is_ok(),
            "Expected TLS config with webpki roots to succeed"
        );
    }

    #[test]
    fn test_tls_config_with_custom_ca() {
        // verify-full with custom CA should use the provided certificate
        let test_ca_pem = generate_test_ca_pem("testca");
        let result = make_tls_config_verifying(true, Some(&test_ca_pem));
        assert!(
            result.is_ok(),
            "Expected TLS config with custom CA to succeed"
        );
    }

    #[test]
    fn test_tls_config_verify_ca_mode() {
        // verify-ca mode (hostname verification disabled)
        let test_ca_pem = generate_test_ca_pem("testca");
        let result = make_tls_config_verifying(false, Some(&test_ca_pem));
        assert!(result.is_ok(), "Expected verify-ca TLS config to succeed");
    }

    #[test]
    fn test_tls_config_with_invalid_ca_fails() {
        let result = make_tls_config_verifying(true, Some("invalid pem data"));
        assert!(
            result.is_err(),
            "Expected TLS config with invalid CA to fail"
        );
    }
}
