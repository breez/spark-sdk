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
/// Reachable only through the explicit `sslmode=no-verify` opt-in, which
/// ensures encryption but not server identity verification.
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

    let provider = Arc::new(default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| {
            PostgresError::Initialization(format!("Failed to configure rustls protocols: {e}"))
        })?;
    let config = if verify_hostname {
        // verify-full: use the standard WebPKI verifier which checks hostname
        builder
            .with_root_certificates(root_store)
            .with_no_client_auth()
    } else {
        // verify-ca: use our custom verifier that only checks the certificate chain
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(CaOnlyVerifier::new(root_store)))
            .with_no_client_auth()
    };

    Ok(config)
}

/// Creates a rustls `ClientConfig` that accepts any server certificate.
/// Used only for the explicit `sslmode=no-verify` opt-in: encrypted
/// connections without server identity verification.
fn make_tls_config_no_verify() -> Result<ClientConfig, PostgresError> {
    Ok(
        ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| {
                PostgresError::Initialization(format!("Failed to configure rustls protocols: {e}"))
            })?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth(),
    )
}

/// Internal representation of SSL modes, including verify-ca, verify-full and
/// no-verify that are not exposed by tokio-postgres's `SslMode` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SslModeExt {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
    NoVerify,
}

/// Extracts the sslmode from a connection string.
/// Handles both key-value format and URI format.
fn parse_sslmode_from_connection_string(conn_str: &str) -> Result<SslModeExt, PostgresError> {
    /// Parses an sslmode value string into an `SslModeExt`.
    /// Surrounding single quotes (legal in the key-value format) are stripped.
    fn parse_sslmode_value(value: &str) -> Result<SslModeExt, PostgresError> {
        match value.trim_matches('\'') {
            "disable" => Ok(SslModeExt::Disable),
            "prefer" => Ok(SslModeExt::Prefer),
            "require" => Ok(SslModeExt::Require),
            "verify-ca" => Ok(SslModeExt::VerifyCa),
            "verify-full" => Ok(SslModeExt::VerifyFull),
            "no-verify" => Ok(SslModeExt::NoVerify),
            // Fail closed: a typo in a verify mode must not silently select a
            // weaker mode.
            _ => Err(PostgresError::Initialization(format!(
                "Unrecognized sslmode value `{value}`; expected one of: disable, prefer, \
                 require, verify-ca, verify-full, no-verify"
            ))),
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
    Ok(SslModeExt::Prefer)
}

/// Rewrites extended sslmode values (`verify-ca`, `verify-full`, `no-verify`)
/// to `require`, the strongest mode tokio-postgres's parser accepts; the actual
/// verification level is enforced by the rustls verifier chosen in
/// `create_pool`. Only the `sslmode` parameter itself is replaced, so a
/// password that happens to contain such a literal is left untouched.
fn driver_connection_string(conn_str: &str, ssl_mode: SslModeExt) -> String {
    let needs_rewrite = matches!(
        ssl_mode,
        SslModeExt::VerifyCa | SslModeExt::VerifyFull | SslModeExt::NoVerify
    );
    if !needs_rewrite {
        return conn_str.to_string();
    }
    let Some(range) = sslmode_param_range(conn_str) else {
        return conn_str.to_string();
    };
    format!(
        "{}sslmode=require{}",
        &conn_str[..range.start],
        &conn_str[range.end..]
    )
}

/// Byte range of the whole `sslmode=<value>` parameter, if present. Mirrors the
/// lookup order of `parse_sslmode_from_connection_string` so the parameter that
/// was parsed is the one replaced.
#[allow(clippy::arithmetic_side_effects)] // in-bounds index math over conn_str
fn sslmode_param_range(conn_str: &str) -> Option<std::ops::Range<usize>> {
    if conn_str.starts_with("postgres://") || conn_str.starts_with("postgresql://") {
        let query_start = conn_str.find('?')? + 1;
        let mut offset = query_start;
        for param in conn_str[query_start..].split('&') {
            if param
                .split_once('=')
                .is_some_and(|(key, _)| key == "sslmode")
            {
                return Some(offset..offset + param.len());
            }
            offset += param.len() + 1;
        }
        None
    } else {
        let mut search_from = 0;
        for token in conn_str.split_whitespace() {
            // Tokens are separated by whitespace only, so the first match at or
            // after the previous token's end is this token's position.
            let start = conn_str[search_from..].find(token)? + search_from;
            search_from = start + token.len();
            if token
                .split_once('=')
                .is_some_and(|(key, _)| key == "sslmode")
            {
                return Some(start..start + token.len());
            }
        }
        None
    }
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

/// Applies TCP liveness defaults so a connection reaped by a managed-`PostgreSQL`
/// NAT/load balancer fails fast instead of lingering half-open: with
/// tokio-postgres's 2h idle keepalive a reaped socket passes deadpool's
/// `is_closed()` recycle check, then the next query hangs ~15 min (`tcp_retries2`)
/// before `os error 110`. Keepalives keep the NAT mapping warm and detect a dead
/// idle peer in ~90s; `tcp_user_timeout` bounds an in-flight query on a dead socket.
///
/// Each value is applied only when the connection string left it unset, so any
/// option passed in the URL wins. `keepalives_idle` is detected on the raw string:
/// tokio-postgres exposes it as a bare `Duration` with no "is set" signal, unlike
/// the `Option` getters backing the other three.
fn apply_tcp_liveness_defaults(connection_string: &str, pg_config: &mut PgConfig) {
    if !connection_string.contains("keepalives_idle") {
        pg_config.keepalives_idle(Duration::from_mins(1));
    }
    if pg_config.get_keepalives_interval().is_none() {
        pg_config.keepalives_interval(Duration::from_secs(10));
    }
    if pg_config.get_keepalives_retries().is_none() {
        pg_config.keepalives_retries(3);
    }
    if pg_config.get_tcp_user_timeout().is_none() {
        pg_config.tcp_user_timeout(Duration::from_secs(30));
    }
}

/// The verifier a given `sslmode` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsChoice {
    /// No TLS connector at all.
    NoTls,
    /// Certificate chain and hostname verification.
    Full,
    /// Certificate chain verification only.
    CaOnly,
    /// No certificate verification.
    NoVerify,
}

/// Maps an `sslmode` to a verifier. Whether TLS is negotiated at all stays with
/// the driver (`prefer`, and absent, remain opportunistic; the rest of the
/// TLS-enabled modes mandate it), but whenever TLS is used the certificate is
/// verified: encrypted-but-unverified is only the explicit `no-verify` opt-in.
fn tls_choice(ssl_mode: SslModeExt) -> TlsChoice {
    match ssl_mode {
        SslModeExt::Disable => TlsChoice::NoTls,
        SslModeExt::Prefer | SslModeExt::Require | SslModeExt::VerifyFull => TlsChoice::Full,
        SslModeExt::VerifyCa => TlsChoice::CaOnly,
        SslModeExt::NoVerify => TlsChoice::NoVerify,
    }
}

/// Creates a `PostgreSQL` connection pool from the given configuration.
///
/// Honors the `sslmode` connection-string parameter:
/// - `disable`: no TLS
/// - absent or `prefer`: TLS when the server supports it, with certificate
///   chain and hostname verification against `root_ca_pem` (pinned) or
///   Mozilla roots
/// - `require` / `verify-full`: TLS mandatory, verified as above
/// - `verify-ca`: TLS mandatory, chain verification only against
///   `root_ca_pem`, which it requires
/// - `no-verify`: TLS mandatory, without certificate verification
///   (explicit opt-in)
///
/// An unrecognized `sslmode` value fails with an initialization error.
pub fn create_pool(config: &PostgresStorageConfig) -> Result<Pool, PostgresError> {
    let ssl_mode = parse_sslmode_from_connection_string(&config.connection_string)?;
    let driver_connection_string = driver_connection_string(&config.connection_string, ssl_mode);
    let mut pg_config: PgConfig = driver_connection_string
        .parse()
        .map_err(|e| PostgresError::Initialization(format!("Invalid connection string: {e}")))?;

    // Guard against parser disagreement: this file's parser scans
    // whitespace-split tokens and takes the first `sslmode`, so a quoted
    // key-value password containing an sslmode literal, spaces around `=`, or
    // a duplicated `sslmode` parameter (tokio-postgres takes the last) can all
    // make the two parsers land on different modes. The verifier selected here
    // must always match the negotiation mode the driver enforces, so any
    // disagreement is an error rather than a silently weaker connection.
    let expected_driver_mode = match ssl_mode {
        SslModeExt::Disable => tokio_postgres::config::SslMode::Disable,
        SslModeExt::Prefer => tokio_postgres::config::SslMode::Prefer,
        // The remaining modes are rewritten to `require` for the driver.
        _ => tokio_postgres::config::SslMode::Require,
    };
    if pg_config.get_ssl_mode() != expected_driver_mode {
        return Err(PostgresError::Initialization(
            "Ambiguous connection string: the sslmode this SDK parsed does not match the one \
             the driver parsed (check for duplicate sslmode parameters, spaces around `=`, or \
             an sslmode literal inside a quoted value)"
                .to_string(),
        ));
    }

    apply_tcp_liveness_defaults(&config.connection_string, &mut pg_config);

    let pool_config = apply_pool_config(config);

    let root_ca_pem = config.root_ca_pem.as_deref();
    let manager = match tls_choice(ssl_mode) {
        TlsChoice::NoTls => deadpool_postgres::Manager::new(pg_config, tokio_postgres::NoTls),
        TlsChoice::Full => {
            let tls = MakeRustlsConnect::new(make_tls_config_verifying(true, root_ca_pem)?);
            deadpool_postgres::Manager::new(pg_config, tls)
        }
        TlsChoice::CaOnly => {
            // Without a pinned CA, chain-only verification accepts a
            // certificate from any public CA for any host, which
            // authenticates nothing.
            let Some(pem) = root_ca_pem else {
                return Err(PostgresError::Initialization(
                    "sslmode=verify-ca requires root_ca_pem; supply the CA to pin, or use \
                     sslmode=require / verify-full for hostname-verified TLS"
                        .to_string(),
                ));
            };
            let tls = MakeRustlsConnect::new(make_tls_config_verifying(false, Some(pem))?);
            deadpool_postgres::Manager::new(pg_config, tls)
        }
        TlsChoice::NoVerify => {
            let tls = MakeRustlsConnect::new(make_tls_config_no_verify()?);
            deadpool_postgres::Manager::new(pg_config, tls)
        }
    };

    Pool::builder(manager)
        .config(pool_config)
        .runtime(deadpool::Runtime::Tokio1)
        .build()
        .map_err(|e| PostgresError::Initialization(e.to_string()))
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

    #[test]
    fn liveness_defaults_applied_when_url_silent() {
        let url = "postgres://u:p@h:5432/db";
        let mut cfg: PgConfig = url.parse().expect("valid");
        apply_tcp_liveness_defaults(url, &mut cfg);
        assert_eq!(cfg.get_keepalives_idle(), Duration::from_mins(1));
        assert_eq!(cfg.get_keepalives_interval(), Some(Duration::from_secs(10)));
        assert_eq!(cfg.get_keepalives_retries(), Some(3));
        assert_eq!(cfg.get_tcp_user_timeout(), Some(&Duration::from_secs(30)));
    }

    #[test]
    fn liveness_defaults_respect_explicit_url_values() {
        let url = "postgres://u:p@h:5432/db\
            ?keepalives_idle=600&keepalives_interval=20&keepalives_retries=9&tcp_user_timeout=5";
        let mut cfg: PgConfig = url.parse().expect("valid");
        apply_tcp_liveness_defaults(url, &mut cfg);
        // Every value supplied in the URL must survive untouched.
        assert_eq!(cfg.get_keepalives_idle(), Duration::from_mins(10));
        assert_eq!(cfg.get_keepalives_interval(), Some(Duration::from_secs(20)));
        assert_eq!(cfg.get_keepalives_retries(), Some(9));
        assert_eq!(cfg.get_tcp_user_timeout(), Some(&Duration::from_secs(5)));
    }

    /// Regression: deadpool's `Pool::builder(...).build()` synchronously rejects
    /// timeouts unless `.runtime(...)` was set on the builder. It does not
    /// detect an ambient tokio runtime, so the explicit call must stay even
    /// when `create_pool` is invoked from within a tokio context.
    #[test]
    fn create_pool_with_timeout_succeeds() {
        let mut cfg = crate::config::PostgresStorageConfig::with_defaults(
            "postgres://postgres:password@127.0.0.1:5432/postgres".to_string(),
        );
        cfg.recycle_timeout_secs = Some(300);
        let pool = create_pool(&cfg);
        assert!(
            pool.is_ok(),
            "create_pool with a timeout should build; got: {:?}",
            pool.err(),
        );
    }

    #[test]
    fn create_pool_accepts_all_ssl_modes() {
        let ca_pem = generate_test_ca_pem("testca");
        for ssl_mode in [
            "disable",
            "prefer",
            "require",
            "verify-ca",
            "verify-full",
            "no-verify",
        ] {
            let mut cfg = crate::config::PostgresStorageConfig::with_defaults(format!(
                "postgres://postgres:password@127.0.0.1:5432/postgres?sslmode={ssl_mode}"
            ));
            cfg.root_ca_pem = Some(ca_pem.clone());
            assert!(
                create_pool(&cfg).is_ok(),
                "create_pool should accept sslmode={ssl_mode}"
            );
        }
    }

    /// Regression guard for the verifier each mode selects: a silent re-mapping
    /// of a verified mode back to `NoVerifier` must fail this test.
    #[test]
    fn ssl_modes_select_expected_verifier() {
        let cases = [
            ("postgres://u:p@h/db?sslmode=disable", TlsChoice::NoTls),
            ("postgres://u:p@h/db", TlsChoice::Full),
            ("postgres://u:p@h/db?sslmode=prefer", TlsChoice::Full),
            ("postgres://u:p@h/db?sslmode=require", TlsChoice::Full),
            ("postgres://u:p@h/db?sslmode=verify-full", TlsChoice::Full),
            ("postgres://u:p@h/db?sslmode=verify-ca", TlsChoice::CaOnly),
            ("postgres://u:p@h/db?sslmode=no-verify", TlsChoice::NoVerify),
            ("host=h user=u sslmode=require", TlsChoice::Full),
            ("host=h user=u sslmode='verify-full'", TlsChoice::Full),
        ];
        for (conn_str, expected) in cases {
            let mode = parse_sslmode_from_connection_string(conn_str).expect("valid sslmode");
            assert_eq!(tls_choice(mode), expected, "for {conn_str}");
        }
    }

    #[test]
    fn unrecognized_sslmode_fails_closed() {
        for conn_str in [
            "postgres://u:p@h/db?sslmode=verify-fll",
            "host=h user=u sslmode=bogus",
        ] {
            let mut cfg = crate::config::PostgresStorageConfig::with_defaults(conn_str.to_string());
            cfg.root_ca_pem = None;
            let result = create_pool(&cfg);
            assert!(
                matches!(result, Err(PostgresError::Initialization(_))),
                "sslmode typo must error, not silently downgrade; got ok for {conn_str}"
            );
        }
    }

    /// Connection strings where this SDK's parser and tokio-postgres land on
    /// different sslmodes must fail instead of silently building a connection
    /// weaker than one of the two parses: an sslmode literal inside a quoted
    /// key-value password, a duplicated sslmode parameter (this SDK takes the
    /// first, tokio-postgres the last), and spaces around `=` (invisible to
    /// this SDK's token scan).
    #[test]
    fn sslmode_parser_disagreement_fails_instead_of_downgrading() {
        for conn_str in [
            "host=h user=u password='a sslmode=disable b' dbname=d",
            "postgres://u:p@h/db?sslmode=require&sslmode=disable",
            "host=h user=u sslmode = disable",
            "host=h user=u sslmode = require",
        ] {
            let cfg = crate::config::PostgresStorageConfig::with_defaults(conn_str.to_string());
            let result = create_pool(&cfg);
            assert!(
                matches!(result, Err(PostgresError::Initialization(_))),
                "parser disagreement must fail closed for {conn_str}; got: {:?}",
                result.map(|_| ()),
            );
        }

        // A genuine sslmode=disable still parses consistently and builds.
        let cfg = crate::config::PostgresStorageConfig::with_defaults(
            "host=h user=u password='a b' dbname=d sslmode=disable".to_string(),
        );
        assert!(create_pool(&cfg).is_ok());
    }

    #[test]
    fn verify_ca_without_root_ca_pem_fails() {
        let cfg = crate::config::PostgresStorageConfig::with_defaults(
            "postgres://u:p@h/db?sslmode=verify-ca".to_string(),
        );
        let result = create_pool(&cfg);
        assert!(
            matches!(result, Err(PostgresError::Initialization(_))),
            "verify-ca without a pinned CA authenticates nothing and must error"
        );
    }

    #[test]
    fn driver_connection_string_rewrites_only_the_sslmode_param() {
        // URI format
        assert_eq!(
            driver_connection_string(
                "postgres://u:p@h/db?application_name=x&sslmode=verify-full&connect_timeout=5",
                SslModeExt::VerifyFull,
            ),
            "postgres://u:p@h/db?application_name=x&sslmode=require&connect_timeout=5"
        );
        assert_eq!(
            driver_connection_string(
                "postgres://u:p@h/db?sslmode=no-verify",
                SslModeExt::NoVerify
            ),
            "postgres://u:p@h/db?sslmode=require"
        );
        // A password containing the literal must not be touched.
        assert_eq!(
            driver_connection_string(
                "host=h password=sslmode=verify-ca dbname=d sslmode=verify-ca",
                SslModeExt::VerifyCa,
            ),
            "host=h password=sslmode=verify-ca dbname=d sslmode=require"
        );
        // Modes the driver understands natively pass through unchanged.
        assert_eq!(
            driver_connection_string("postgres://u:p@h/db?sslmode=require", SslModeExt::Require),
            "postgres://u:p@h/db?sslmode=require"
        );
    }
}
