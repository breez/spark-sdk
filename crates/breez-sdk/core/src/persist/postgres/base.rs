//! Shared `PostgreSQL` infrastructure for storage implementations.
//!
//! This module contains common configuration, connection pooling,
//! TLS setup, and error mapping used by both `PostgresStorage` and `PostgresTreeStore`.

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

use crate::persist::StorageError;

/// Creates a `PostgresStorageConfig` with the given connection string and default pool settings.
///
/// This is a convenience function for creating a config with sensible defaults from deadpool.
/// Use this instead of manually constructing `PostgresStorageConfig` when you want defaults.
///
/// Default values:
/// - `max_pool_size`: `num_cpus * 4`
/// - `wait_timeout_secs`: `None` (wait indefinitely)
/// - `create_timeout_secs`: `None` (no timeout)
/// - `recycle_timeout_secs`: `None` (no timeout)
/// - `queue_mode`: FIFO
/// - `root_ca_pem`: `None` (uses Mozilla's root certificate store)
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn default_postgres_storage_config(connection_string: String) -> PostgresStorageConfig {
    PostgresStorageConfig::with_defaults(connection_string)
}

/// Queue mode for the connection pool.
///
/// Determines the order in which connections are retrieved from the pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PoolQueueMode {
    /// First In, First Out (default).
    /// Connections are used in the order they were returned to the pool.
    /// Spreads load evenly across all connections.
    #[default]
    Fifo,
    /// Last In, First Out.
    /// Most recently returned connections are used first.
    /// Keeps fewer connections "hot" and allows idle connections to close sooner.
    Lifo,
}

impl From<PoolQueueMode> for deadpool::managed::QueueMode {
    fn from(mode: PoolQueueMode) -> Self {
        match mode {
            PoolQueueMode::Fifo => deadpool::managed::QueueMode::Fifo,
            PoolQueueMode::Lifo => deadpool::managed::QueueMode::Lifo,
        }
    }
}

impl From<deadpool::managed::QueueMode> for PoolQueueMode {
    fn from(mode: deadpool::managed::QueueMode) -> Self {
        match mode {
            deadpool::managed::QueueMode::Fifo => PoolQueueMode::Fifo,
            deadpool::managed::QueueMode::Lifo => PoolQueueMode::Lifo,
        }
    }
}

/// Returns the default pool configuration values from deadpool.
fn default_pool_config() -> deadpool_postgres::PoolConfig {
    deadpool_postgres::PoolConfig::default()
}

/// Configuration for `PostgreSQL` storage connection pool.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PostgresStorageConfig {
    /// `PostgreSQL` connection string (key-value or URI format).
    ///
    /// Supported formats:
    /// - Key-value: `host=localhost user=postgres dbname=spark sslmode=require`
    /// - URI: `postgres://user:password@host:port/dbname?sslmode=require`
    pub connection_string: String,

    /// Maximum number of connections in the pool.
    /// Default: `num_cpus * 4` (from deadpool).
    pub max_pool_size: u32,

    /// Timeout in seconds waiting for a connection from the pool.
    /// `None` means wait indefinitely.
    pub wait_timeout_secs: Option<u64>,

    /// Timeout in seconds for establishing a new connection.
    /// `None` means no timeout.
    pub create_timeout_secs: Option<u64>,

    /// Timeout in seconds before recycling an idle connection.
    /// `None` means connections are not recycled based on idle time.
    pub recycle_timeout_secs: Option<u64>,

    /// Queue mode for retrieving connections from the pool.
    /// Default: FIFO.
    pub queue_mode: PoolQueueMode,

    /// Custom CA certificate(s) in PEM format for server verification.
    /// If `None`, uses Mozilla's root certificate store (via webpki-roots).
    /// Only used with `sslmode=verify-ca` or `sslmode=verify-full`.
    pub root_ca_pem: Option<String>,
}

impl PostgresStorageConfig {
    /// Creates a new configuration with the given connection string and pool defaults from deadpool.
    ///
    /// Default values:
    /// - `max_pool_size`: `num_cpus * 4`
    /// - `wait_timeout_secs`: `None` (wait indefinitely)
    /// - `create_timeout_secs`: `None` (no timeout)
    /// - `recycle_timeout_secs`: `None` (no timeout)
    /// - `queue_mode`: FIFO
    #[must_use]
    pub fn with_defaults(connection_string: impl Into<String>) -> Self {
        let defaults = default_pool_config();
        Self {
            connection_string: connection_string.into(),
            max_pool_size: u32::try_from(defaults.max_size).unwrap_or(u32::MAX),
            wait_timeout_secs: defaults.timeouts.wait.map(|d| d.as_secs()),
            create_timeout_secs: defaults.timeouts.create.map(|d| d.as_secs()),
            recycle_timeout_secs: defaults.timeouts.recycle.map(|d| d.as_secs()),
            queue_mode: defaults.queue_mode.into(),
            root_ca_pem: None,
        }
    }
}

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
pub(super) fn parse_pem_to_root_store(pem: &str) -> Result<RootCertStore, StorageError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            StorageError::InitializationError(format!("Failed to parse PEM certificates: {e}"))
        })?;

    if certs.is_empty() {
        return Err(StorageError::InitializationError(
            "No valid certificates found in PEM data".to_string(),
        ));
    }

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store.add(cert).map_err(|e| {
            StorageError::InitializationError(format!("Failed to add certificate to store: {e}"))
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
pub(super) fn make_tls_config_verifying(
    verify_hostname: bool,
    custom_ca: Option<&str>,
) -> Result<ClientConfig, StorageError> {
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
///
/// This is a shared helper used by both `PostgresStorage` and `PostgresTreeStore`.
pub fn create_pool(config: &PostgresStorageConfig) -> Result<Pool, StorageError> {
    let pg_config: PgConfig = config.connection_string.parse().map_err(|e| {
        StorageError::InitializationError(format!("Invalid connection string: {e}"))
    })?;

    let ssl_mode = parse_sslmode_from_connection_string(&config.connection_string);
    let pool_config = apply_pool_config(config);

    match ssl_mode {
        SslModeExt::Disable => {
            let manager = deadpool_postgres::Manager::new(pg_config, tokio_postgres::NoTls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| StorageError::InitializationError(e.to_string()))
        }
        SslModeExt::Prefer | SslModeExt::Require => {
            let tls_config = make_tls_config();
            let tls = MakeRustlsConnect::new(tls_config);
            let manager = deadpool_postgres::Manager::new(pg_config, tls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| StorageError::InitializationError(e.to_string()))
        }
        SslModeExt::VerifyCa => {
            let tls_config = make_tls_config_verifying(false, config.root_ca_pem.as_deref())?;
            let tls = MakeRustlsConnect::new(tls_config);
            let manager = deadpool_postgres::Manager::new(pg_config, tls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| StorageError::InitializationError(e.to_string()))
        }
        SslModeExt::VerifyFull => {
            let tls_config = make_tls_config_verifying(true, config.root_ca_pem.as_deref())?;
            let tls = MakeRustlsConnect::new(tls_config);
            let manager = deadpool_postgres::Manager::new(pg_config, tls);
            Pool::builder(manager)
                .config(pool_config)
                .build()
                .map_err(|e| StorageError::InitializationError(e.to_string()))
        }
    }
}

/// Maps a deadpool-postgres pool error to the appropriate `StorageError`.
/// Pool errors (exhaustion, timeout) are connection-related.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_pool_error(e: deadpool_postgres::PoolError) -> StorageError {
    StorageError::Connection(e.to_string())
}

/// Maps a tokio-postgres database error to the appropriate `StorageError`.
/// Connection-class errors (Class 08) and closed connections are mapped to `Connection`,
/// other errors are mapped to `Implementation`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_db_error(e: tokio_postgres::Error) -> StorageError {
    // Check if the connection is closed
    if e.is_closed() {
        return StorageError::Connection(e.to_string());
    }
    // Check SQL state codes for connection errors (Class 08)
    if let Some(code) = e.code()
        && code.code().starts_with("08")
    {
        return StorageError::Connection(e.to_string());
    }
    StorageError::Implementation(e.to_string())
}

impl From<tokio_postgres::Error> for StorageError {
    fn from(value: tokio_postgres::Error) -> Self {
        map_db_error(value)
    }
}

/// Runs database migrations with version tracking and concurrency control.
///
/// This function:
/// - Acquires an advisory lock (derived from `migration_lock_{table_name}`) to prevent concurrent migrations
/// - Creates a migrations tracking table if it doesn't exist
/// - Applies only new migrations (based on version number)
/// - Commits all changes in a single transaction
///
/// # Arguments
/// * `pool` - The connection pool to use
/// * `migrations_table` - Name of the table to track migration versions (e.g., `schema_migrations`)
/// * `migrations` - List of migrations, where each migration is a list of SQL statements
#[allow(clippy::arithmetic_side_effects)]
pub(super) async fn run_migrations(
    pool: &Pool,
    migrations_table: &str,
    migrations: &[&[&str]],
) -> Result<(), StorageError> {
    let mut client = pool.get().await.map_err(map_pool_error)?;

    // Generate a unique advisory lock ID from a descriptive lock name
    let lock_name = format!("migration_lock_{migrations_table}");
    let lock_id: i64 = lock_name.bytes().map(i64::from).sum();

    // Run all migrations in a single transaction with a transaction-level advisory lock.
    // pg_advisory_xact_lock is automatically released on commit/rollback, making it safe
    // with connection pools (no risk of leaked locks if the task is cancelled or panics).
    let tx = client.transaction().await.map_err(map_db_error)?;

    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&lock_id])
        .await
        .map_err(|e| {
            StorageError::InitializationError(format!("Failed to acquire migration lock: {e}"))
        })?;

    // Create migrations table if it doesn't exist
    // Note: table names cannot be parameterized in PostgreSQL, so we use format!
    let create_table_sql = format!(
        "CREATE TABLE IF NOT EXISTS {migrations_table} (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMPTZ DEFAULT NOW()
        )"
    );
    tx.execute(&create_table_sql, &[])
        .await
        .map_err(map_db_error)?;

    // Get current version
    let get_version_sql = format!("SELECT COALESCE(MAX(version), 0) FROM {migrations_table}");
    let current_version: i32 = tx
        .query_opt(&get_version_sql, &[])
        .await
        .map_err(map_db_error)?
        .map_or(0, |row| row.get(0));

    for (i, migration) in migrations.iter().enumerate() {
        let version = i32::try_from(i + 1).unwrap_or(i32::MAX);
        if version > current_version {
            for statement in *migration {
                tx.execute(*statement, &[]).await.map_err(|e| {
                    StorageError::Implementation(format!("Migration {version} failed: {e}"))
                })?;
            }
            let insert_version_sql =
                format!("INSERT INTO {migrations_table} (version) VALUES ($1)");
            tx.execute(&insert_version_sql, &[&version])
                .await
                .map_err(map_db_error)?;
        }
    }

    tx.commit().await.map_err(map_db_error)?;

    Ok(())
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
            matches!(err, StorageError::InitializationError(_)),
            "Expected InitializationError"
        );
    }

    #[test]
    fn test_parse_empty_pem() {
        let empty_pem = "";
        let result = parse_pem_to_root_store(empty_pem);
        assert!(result.is_err(), "Expected empty PEM to fail");
        let err = result.unwrap_err();
        match err {
            StorageError::InitializationError(msg) => {
                assert!(
                    msg.contains("No valid certificates"),
                    "Expected 'No valid certificates' error message, got: {msg}"
                );
            }
            _ => panic!("Expected InitializationError"),
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
