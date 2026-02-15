use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::Pool;
use macros::async_trait;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject};
use rustls::server::ParsedCertificate;
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
};
use tokio_postgres::{Config as PgConfig, Row, types::ToSql};
use tokio_postgres_rustls::MakeRustlsConnect;
use webpki_roots::TLS_SERVER_ROOTS;

use crate::{
    AssetFilter, ConversionInfo, DepositInfo, ListPaymentsRequest, LnurlPayInfo,
    LnurlReceiveMetadata, LnurlWithdrawInfo, PaymentDetails, PaymentDetailsFilter, PaymentMethod,
    error::DepositClaimError,
    persist::{PaymentMetadata, SetLnurlMetadataItem, UpdateDepositPayload},
    sync_storage::{
        IncomingChange, OutgoingChange, Record, RecordChange, RecordId, UnversionedRecordChange,
    },
};

use tracing::warn;

use super::{Payment, Storage, StorageError};

/// Advisory lock ID for migrations.
/// Derived from ASCII bytes of "MIGR" (`0x4D49_4752`).
const MIGRATION_LOCK_ID: i64 = 0x4D49_4752;

/// Creates a `PostgreSQL` storage instance for use with the SDK builder.
///
/// Returns a `Storage` trait object backed by the `PostgreSQL` connection pool.
///
/// # Arguments
///
/// * `config` - Configuration for the `PostgreSQL` connection pool
///
/// # Example
///
/// ```ignore
/// use breez_sdk_core::{create_postgres_storage, default_postgres_storage_config};
///
/// let storage = create_postgres_storage(default_postgres_storage_config(
///     "host=localhost user=postgres dbname=spark".to_string()
/// )).await?;
///
/// let sdk = SdkBuilder::new(config, seed)
///     .with_storage(storage)
///     .build()
///     .await?;
/// ```
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn create_postgres_storage(
    config: PostgresStorageConfig,
) -> Result<Arc<dyn Storage>, StorageError> {
    Ok(Arc::new(PostgresStorage::new(config).await?))
}

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
fn parse_pem_to_root_store(pem: &str) -> Result<RootCertStore, StorageError> {
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
fn make_tls_config_verifying(
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

/// PostgreSQL-based storage implementation using connection pooling
pub struct PostgresStorage {
    pool: Pool,
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

impl PostgresStorage {
    /// Creates a new `PostgresStorage` with a connection pool.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the `PostgreSQL` connection pool
    ///
    /// # Connection String Formats
    ///
    /// - Key-value: `host=localhost user=postgres dbname=spark sslmode=require`
    /// - URI: `postgres://user:password@host:port/dbname?sslmode=require`
    ///
    /// # Supported `sslmode` values
    ///
    /// - `disable` - No TLS (default if not specified)
    /// - `prefer` - Try TLS, fall back to plaintext if unavailable
    /// - `require` - TLS required, but accept any server certificate
    /// - `verify-ca` - TLS required, verify server certificate is signed by a trusted CA
    /// - `verify-full` - TLS required, verify CA and that server hostname matches certificate
    ///
    /// # Returns
    ///
    /// A new `PostgresStorage` instance or an error
    pub async fn new(config: PostgresStorageConfig) -> Result<Self, StorageError> {
        let pg_config: PgConfig = config.connection_string.parse().map_err(|e| {
            StorageError::InitializationError(format!("Invalid connection string: {e}"))
        })?;

        // Parse sslmode ourselves since tokio-postgres doesn't expose verify-ca/verify-full
        let ssl_mode = parse_sslmode_from_connection_string(&config.connection_string);
        let pool_config = apply_pool_config(&config);
        let pool = match ssl_mode {
            SslModeExt::Disable => {
                let manager = deadpool_postgres::Manager::new(pg_config, tokio_postgres::NoTls);
                Pool::builder(manager)
                    .config(pool_config)
                    .build()
                    .map_err(|e| StorageError::InitializationError(e.to_string()))?
            }
            SslModeExt::Prefer | SslModeExt::Require => {
                let tls_config = make_tls_config();
                let tls = MakeRustlsConnect::new(tls_config);
                let manager = deadpool_postgres::Manager::new(pg_config, tls);
                Pool::builder(manager)
                    .config(pool_config)
                    .build()
                    .map_err(|e| StorageError::InitializationError(e.to_string()))?
            }
            SslModeExt::VerifyCa => {
                let tls_config = make_tls_config_verifying(false, config.root_ca_pem.as_deref())?;
                let tls = MakeRustlsConnect::new(tls_config);
                let manager = deadpool_postgres::Manager::new(pg_config, tls);
                Pool::builder(manager)
                    .config(pool_config)
                    .build()
                    .map_err(|e| StorageError::InitializationError(e.to_string()))?
            }
            SslModeExt::VerifyFull => {
                let tls_config = make_tls_config_verifying(true, config.root_ca_pem.as_deref())?;
                let tls = MakeRustlsConnect::new(tls_config);
                let manager = deadpool_postgres::Manager::new(pg_config, tls);
                Pool::builder(manager)
                    .config(pool_config)
                    .build()
                    .map_err(|e| StorageError::InitializationError(e.to_string()))?
            }
        };

        let storage = Self { pool };
        storage.migrate().await?;
        Ok(storage)
    }

    #[allow(clippy::arithmetic_side_effects)]
    async fn migrate(&self) -> Result<(), StorageError> {
        let mut client = self.pool.get().await.map_err(map_pool_error)?;

        // Run all migrations in a single transaction with a transaction-level advisory lock.
        // pg_advisory_xact_lock is automatically released on commit/rollback, making it safe
        // with connection pools (no risk of leaked locks if the task is cancelled or panics).
        let tx = client.transaction().await.map_err(map_db_error)?;

        tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])
            .await
            .map_err(|e| {
                StorageError::InitializationError(format!("Failed to acquire migration lock: {e}"))
            })?;

        // Create migrations table if it doesn't exist
        tx.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMPTZ DEFAULT NOW()
            )",
            &[],
        )
        .await
        .map_err(map_db_error)?;

        // Get current version
        let current_version: i32 = tx
            .query_opt(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                &[],
            )
            .await
            .map_err(map_db_error)?
            .map_or(0, |row| row.get(0));

        let migrations = Self::migrations();

        for (i, migration) in migrations.iter().enumerate() {
            let version = i32::try_from(i + 1).unwrap_or(i32::MAX);
            if version > current_version {
                for statement in *migration {
                    tx.execute(*statement, &[]).await.map_err(|e| {
                        StorageError::Implementation(format!("Migration {version} failed: {e}"))
                    })?;
                }
                tx.execute(
                    "INSERT INTO schema_migrations (version) VALUES ($1)",
                    &[&version],
                )
                .await
                .map_err(map_db_error)?;
            }
        }

        tx.commit().await.map_err(map_db_error)?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn migrations() -> Vec<&'static [&'static str]> {
        vec![
            // Migration 1: Core tables
            &[
                "CREATE TABLE IF NOT EXISTS payments (
                    id TEXT PRIMARY KEY,
                    payment_type TEXT NOT NULL,
                    status TEXT NOT NULL,
                    amount TEXT NOT NULL,
                    fees TEXT NOT NULL,
                    timestamp BIGINT NOT NULL,
                    method TEXT,
                    withdraw_tx_id TEXT,
                    deposit_tx_id TEXT,
                    spark BOOLEAN
                )",
                "CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )",
                "CREATE TABLE IF NOT EXISTS unclaimed_deposits (
                    txid TEXT NOT NULL,
                    vout INTEGER NOT NULL,
                    amount_sats BIGINT,
                    claim_error JSONB,
                    refund_tx TEXT,
                    refund_tx_id TEXT,
                    PRIMARY KEY (txid, vout)
                )",
                "CREATE TABLE IF NOT EXISTS payment_metadata (
                    payment_id TEXT PRIMARY KEY,
                    parent_payment_id TEXT,
                    lnurl_pay_info JSONB,
                    lnurl_withdraw_info JSONB,
                    lnurl_description TEXT,
                    conversion_info JSONB
                )",
                "CREATE TABLE IF NOT EXISTS payment_details_lightning (
                    payment_id TEXT PRIMARY KEY,
                    invoice TEXT NOT NULL,
                    payment_hash TEXT NOT NULL,
                    destination_pubkey TEXT NOT NULL,
                    description TEXT,
                    preimage TEXT
                )",
                "CREATE TABLE IF NOT EXISTS payment_details_token (
                    payment_id TEXT PRIMARY KEY,
                    metadata JSONB NOT NULL,
                    tx_hash TEXT NOT NULL,
                    invoice_details JSONB
                )",
                "CREATE TABLE IF NOT EXISTS payment_details_spark (
                    payment_id TEXT PRIMARY KEY,
                    invoice_details JSONB,
                    htlc_details JSONB
                )",
                "CREATE TABLE IF NOT EXISTS lnurl_receive_metadata (
                    payment_hash TEXT PRIMARY KEY,
                    nostr_zap_request TEXT,
                    nostr_zap_receipt TEXT,
                    sender_comment TEXT
                )",
            ],
            // Migration 2: Sync tables
            &[
                // sync_revision: tracks the last committed revision (from server-acknowledged
                // or server-received records). Does NOT include pending outgoing queue ids.
                // sync_outgoing.revision stores a local queue id for ordering/de-duplication only.
                "CREATE TABLE IF NOT EXISTS sync_revision (
                    id INTEGER PRIMARY KEY DEFAULT 1,
                    revision BIGINT NOT NULL DEFAULT 0,
                    CHECK (id = 1)
                )",
                "INSERT INTO sync_revision (id, revision) VALUES (1, 0) ON CONFLICT (id) DO NOTHING",
                "CREATE TABLE IF NOT EXISTS sync_outgoing (
                    record_type TEXT NOT NULL,
                    data_id TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    commit_time BIGINT NOT NULL,
                    updated_fields_json JSONB NOT NULL,
                    revision BIGINT NOT NULL
                )",
                "CREATE INDEX IF NOT EXISTS idx_sync_outgoing_data_id_record_type ON sync_outgoing(record_type, data_id)",
                "CREATE TABLE IF NOT EXISTS sync_state (
                    record_type TEXT NOT NULL,
                    data_id TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    commit_time BIGINT NOT NULL,
                    data JSONB NOT NULL,
                    revision BIGINT NOT NULL,
                    PRIMARY KEY(record_type, data_id)
                )",
                "CREATE TABLE IF NOT EXISTS sync_incoming (
                    record_type TEXT NOT NULL,
                    data_id TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    commit_time BIGINT NOT NULL,
                    data JSONB NOT NULL,
                    revision BIGINT NOT NULL,
                    PRIMARY KEY(record_type, data_id, revision)
                )",
                "CREATE INDEX IF NOT EXISTS idx_sync_incoming_revision ON sync_incoming(revision)",
            ],
            // Migration 3: Indexes
            &[
                "CREATE INDEX IF NOT EXISTS idx_payments_timestamp ON payments(timestamp)",
                "CREATE INDEX IF NOT EXISTS idx_payments_payment_type ON payments(payment_type)",
                "CREATE INDEX IF NOT EXISTS idx_payments_status ON payments(status)",
                "CREATE INDEX IF NOT EXISTS idx_payment_details_lightning_invoice ON payment_details_lightning(invoice)",
                "CREATE INDEX IF NOT EXISTS idx_payment_metadata_parent ON payment_metadata(parent_payment_id)",
            ],
            // Migration 4: Add tx_type to token payments
            &[
                "ALTER TABLE payment_details_token ADD COLUMN tx_type TEXT NOT NULL DEFAULT 'transfer'",
            ],
            // Migration 5: Clear sync tables to force re-sync
            &[
                "DELETE FROM sync_outgoing",
                "DELETE FROM sync_incoming",
                "DELETE FROM sync_state",
                "UPDATE sync_revision SET revision = 0",
                "DELETE FROM settings WHERE key = 'sync_initial_complete'",
            ],
        ]
    }
}

/// Maps a deadpool-postgres pool error to the appropriate `StorageError`.
/// Pool errors (exhaustion, timeout) are connection-related.
#[allow(clippy::needless_pass_by_value)]
fn map_pool_error(e: deadpool_postgres::PoolError) -> StorageError {
    StorageError::Connection(e.to_string())
}

/// Maps a tokio-postgres database error to the appropriate `StorageError`.
/// Connection-class errors (Class 08) and closed connections are mapped to `Connection`,
/// other errors are mapped to `Implementation`.
#[allow(clippy::needless_pass_by_value)]
fn map_db_error(e: tokio_postgres::Error) -> StorageError {
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

/// Converts an optional serializable value to an optional `serde_json::Value` for JSONB storage.
fn to_json_opt<T: serde::Serialize>(
    value: Option<&T>,
) -> Result<Option<serde_json::Value>, StorageError> {
    value
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Converts an optional `serde_json::Value` to an optional deserialized type.
fn from_json_opt<T: serde::de::DeserializeOwned>(
    value: Option<serde_json::Value>,
) -> Result<Option<T>, StorageError> {
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

#[async_trait]
impl Storage for PostgresStorage {
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        // Build WHERE clauses based on filters
        let mut where_clauses = Vec::new();
        let mut params: Vec<Box<dyn ToSql + Sync + Send>> = Vec::new();
        let mut param_idx = 1;

        // Filter by payment type
        if let Some(ref type_filter) = request.type_filter
            && !type_filter.is_empty()
        {
            let placeholders: Vec<String> = type_filter
                .iter()
                .map(|_| {
                    let placeholder = format!("${param_idx}");
                    param_idx += 1;
                    placeholder
                })
                .collect();
            where_clauses.push(format!("p.payment_type IN ({})", placeholders.join(", ")));
            for payment_type in type_filter {
                params.push(Box::new(payment_type.to_string()));
            }
        }

        // Filter by status
        if let Some(ref status_filter) = request.status_filter
            && !status_filter.is_empty()
        {
            let placeholders: Vec<String> = status_filter
                .iter()
                .map(|_| {
                    let placeholder = format!("${param_idx}");
                    param_idx += 1;
                    placeholder
                })
                .collect();
            where_clauses.push(format!("p.status IN ({})", placeholders.join(", ")));
            for status in status_filter {
                params.push(Box::new(status.to_string()));
            }
        }

        // Filter by timestamp range
        if let Some(from_timestamp) = request.from_timestamp {
            where_clauses.push(format!("p.timestamp >= ${param_idx}"));
            param_idx += 1;
            params.push(Box::new(i64::try_from(from_timestamp)?));
        }

        if let Some(to_timestamp) = request.to_timestamp {
            where_clauses.push(format!("p.timestamp < ${param_idx}"));
            param_idx += 1;
            params.push(Box::new(i64::try_from(to_timestamp)?));
        }

        // Filter by asset
        if let Some(ref asset_filter) = request.asset_filter {
            match asset_filter {
                AssetFilter::Bitcoin => {
                    where_clauses.push("t.metadata IS NULL".to_string());
                }
                AssetFilter::Token { token_identifier } => {
                    where_clauses.push("t.metadata IS NOT NULL".to_string());
                    if let Some(identifier) = token_identifier {
                        where_clauses
                            .push(format!("t.metadata::jsonb->>'identifier' = ${param_idx}"));
                        param_idx += 1;
                        params.push(Box::new(identifier.clone()));
                    }
                }
            }
        }

        // Filter by payment details
        if let Some(ref payment_details_filter) = request.payment_details_filter {
            let mut all_payment_details_clauses = Vec::new();
            for payment_details_filter in payment_details_filter {
                let mut payment_details_clauses = Vec::new();
                // Filter by Spark HTLC status
                if let PaymentDetailsFilter::Spark {
                    htlc_status: Some(htlc_statuses),
                    ..
                } = payment_details_filter
                    && !htlc_statuses.is_empty()
                {
                    let placeholders: Vec<String> = htlc_statuses
                        .iter()
                        .map(|_| {
                            let placeholder = format!("${param_idx}");
                            param_idx += 1;
                            placeholder
                        })
                        .collect();
                    payment_details_clauses.push(format!(
                        "s.htlc_details::jsonb->>'status' IN ({})",
                        placeholders.join(", ")
                    ));
                    for htlc_status in htlc_statuses {
                        params.push(Box::new(htlc_status.to_string()));
                    }
                }
                // Filter by conversion info presence
                if let PaymentDetailsFilter::Spark {
                    conversion_refund_needed: Some(conversion_refund_needed),
                    ..
                }
                | PaymentDetailsFilter::Token {
                    conversion_refund_needed: Some(conversion_refund_needed),
                    ..
                } = payment_details_filter
                {
                    let type_check = match payment_details_filter {
                        PaymentDetailsFilter::Spark { .. } => "p.spark = true",
                        PaymentDetailsFilter::Token { .. } => "p.spark IS NULL",
                    };
                    let refund_needed = if *conversion_refund_needed {
                        "= 'RefundNeeded'"
                    } else {
                        "!= 'RefundNeeded'"
                    };
                    payment_details_clauses.push(format!(
                        "{type_check} AND pm.conversion_info IS NOT NULL AND
                         pm.conversion_info::jsonb->>'status' {refund_needed}"
                    ));
                }
                // Filter by token transaction hash
                if let PaymentDetailsFilter::Token {
                    tx_hash: Some(tx_hash),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push(format!("t.tx_hash = ${param_idx}"));
                    param_idx += 1;
                    params.push(Box::new(tx_hash.clone()));
                }
                // Filter by token transaction type
                if let PaymentDetailsFilter::Token {
                    tx_type: Some(tx_type),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push(format!("t.tx_type = ${param_idx}"));
                    param_idx += 1;
                    params.push(Box::new(tx_type.to_string()));
                }

                if !payment_details_clauses.is_empty() {
                    all_payment_details_clauses
                        .push(format!("({})", payment_details_clauses.join(" AND ")));
                }
            }

            if !all_payment_details_clauses.is_empty() {
                where_clauses.push(format!("({})", all_payment_details_clauses.join(" OR ")));
            }
        }

        // Exclude child payments
        where_clauses.push("pm.parent_payment_id IS NULL".to_string());

        // Build the WHERE clause
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        // Determine sort order
        let order_direction = if request.sort_ascending.unwrap_or(false) {
            "ASC"
        } else {
            "DESC"
        };

        let limit = i64::from(request.limit.unwrap_or(u32::MAX));
        let offset = i64::from(request.offset.unwrap_or(0));

        let offset_idx = param_idx + 1;
        let query = format!(
            "{SELECT_PAYMENT_SQL} {where_sql} ORDER BY p.timestamp {order_direction} LIMIT ${param_idx} OFFSET ${offset_idx}"
        );

        params.push(Box::new(limit));
        params.push(Box::new(offset));

        let param_refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();

        let rows = client
            .query(&query, &param_refs)
            .await
            .map_err(map_db_error)?;

        let mut payments = Vec::new();
        for row in rows {
            payments.push(map_payment(&row)?);
        }
        Ok(payments)
    }

    #[allow(clippy::too_many_lines)]
    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let mut client = self.pool.get().await.map_err(map_pool_error)?;

        let tx = client.transaction().await.map_err(map_db_error)?;

        // Insert or update main payment record
        tx.execute(
            "INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(id) DO UPDATE SET
                    payment_type = EXCLUDED.payment_type,
                    status = EXCLUDED.status,
                    amount = EXCLUDED.amount,
                    fees = EXCLUDED.fees,
                    timestamp = EXCLUDED.timestamp,
                    method = EXCLUDED.method",
            &[
                &payment.id,
                &payment.payment_type.to_string(),
                &payment.status.to_string(),
                &payment.amount.to_string(),
                &payment.fees.to_string(),
                &i64::try_from(payment.timestamp)?,
                &Some(payment.method.to_string()),
            ],
        )
        .await?;

        match payment.details {
            Some(PaymentDetails::Withdraw { tx_id }) => {
                tx.execute(
                    "UPDATE payments SET withdraw_tx_id = $1 WHERE id = $2",
                    &[&tx_id, &payment.id],
                )
                .await?;
            }
            Some(PaymentDetails::Deposit { tx_id }) => {
                tx.execute(
                    "UPDATE payments SET deposit_tx_id = $1 WHERE id = $2",
                    &[&tx_id, &payment.id],
                )
                .await?;
            }
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                ..
            }) => {
                tx.execute(
                    "UPDATE payments SET spark = true WHERE id = $1",
                    &[&payment.id],
                )
                .await?;
                if invoice_details.is_some() || htlc_details.is_some() {
                    let invoice_json = to_json_opt(invoice_details.as_ref())?;
                    let htlc_json = to_json_opt(htlc_details.as_ref())?;
                    tx.execute(
                        "INSERT INTO payment_details_spark (payment_id, invoice_details, htlc_details)
                             VALUES ($1, $2, $3)
                             ON CONFLICT(payment_id) DO UPDATE SET
                                invoice_details = COALESCE(EXCLUDED.invoice_details, payment_details_spark.invoice_details),
                                htlc_details = COALESCE(EXCLUDED.htlc_details, payment_details_spark.htlc_details)",
                        &[&payment.id, &invoice_json, &htlc_json],
                    )
                    .await?;
                }
            }
            Some(PaymentDetails::Token {
                metadata,
                tx_hash,
                tx_type,
                invoice_details,
                ..
            }) => {
                let metadata_json = serde_json::to_value(&metadata)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                let invoice_json = to_json_opt(invoice_details.as_ref())?;
                tx.execute(
                    "INSERT INTO payment_details_token (payment_id, metadata, tx_hash, tx_type, invoice_details)
                         VALUES ($1, $2, $3, $4, $5)
                         ON CONFLICT(payment_id) DO UPDATE SET
                            metadata = EXCLUDED.metadata,
                            tx_hash = EXCLUDED.tx_hash,
                            tx_type = EXCLUDED.tx_type,
                            invoice_details = COALESCE(EXCLUDED.invoice_details, payment_details_token.invoice_details)",
                    &[&payment.id, &metadata_json, &tx_hash, &tx_type.to_string(), &invoice_json],
                )
                .await?;
            }
            Some(PaymentDetails::Lightning {
                invoice,
                payment_hash,
                destination_pubkey,
                description,
                preimage,
                ..
            }) => {
                tx.execute(
                    "INSERT INTO payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, description, preimage)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT(payment_id) DO UPDATE SET
                            invoice = EXCLUDED.invoice,
                            payment_hash = EXCLUDED.payment_hash,
                            destination_pubkey = EXCLUDED.destination_pubkey,
                            description = EXCLUDED.description,
                            preimage = COALESCE(EXCLUDED.preimage, payment_details_lightning.preimage)",
                    &[&payment.id, &invoice, &payment_hash, &destination_pubkey, &description, &preimage],
                )
                .await?;
            }
            None => {}
        }

        tx.commit().await.map_err(map_db_error)?;

        Ok(())
    }

    async fn insert_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let lnurl_pay_info_json = to_json_opt(metadata.lnurl_pay_info.as_ref())?;
        let lnurl_withdraw_info_json = to_json_opt(metadata.lnurl_withdraw_info.as_ref())?;
        let conversion_info_json = to_json_opt(metadata.conversion_info.as_ref())?;

        client
            .execute(
                "INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(payment_id) DO UPDATE SET
                    parent_payment_id = COALESCE(EXCLUDED.parent_payment_id, payment_metadata.parent_payment_id),
                    lnurl_pay_info = COALESCE(EXCLUDED.lnurl_pay_info, payment_metadata.lnurl_pay_info),
                    lnurl_withdraw_info = COALESCE(EXCLUDED.lnurl_withdraw_info, payment_metadata.lnurl_withdraw_info),
                    lnurl_description = COALESCE(EXCLUDED.lnurl_description, payment_metadata.lnurl_description),
                    conversion_info = COALESCE(EXCLUDED.conversion_info, payment_metadata.conversion_info)",
                &[
                    &payment_id,
                    &metadata.parent_payment_id,
                    &lnurl_pay_info_json,
                    &lnurl_withdraw_info_json,
                    &metadata.lnurl_description,
                    &conversion_info_json,
                ],
            )
            .await?;

        Ok(())
    }

    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        client
            .execute(
                "INSERT INTO settings (key, value) VALUES ($1, $2)
                 ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value",
                &[&key, &value],
            )
            .await?;

        Ok(())
    }

    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let row = client
            .query_opt("SELECT value FROM settings WHERE key = $1", &[&key])
            .await?;

        Ok(row.map(|r| r.get(0)))
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        client
            .execute("DELETE FROM settings WHERE key = $1", &[&key])
            .await?;

        Ok(())
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.id = $1");
        let row = client
            .query_one(&query, &[&id])
            .await
            .map_err(map_db_error)?;
        map_payment(&row)
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE l.invoice = $1");
        let row = client.query_opt(&query, &[&invoice]).await?;

        match row {
            Some(r) => Ok(Some(map_payment(&r)?)),
            None => Ok(None),
        }
    }

    #[allow(clippy::arithmetic_side_effects)]
    async fn get_payments_by_parent_ids(
        &self,
        parent_payment_ids: Vec<String>,
    ) -> Result<HashMap<String, Vec<Payment>>, StorageError> {
        if parent_payment_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let client = self.pool.get().await.map_err(map_pool_error)?;

        // Early exit if no related payments exist
        let has_related: bool = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM payment_metadata WHERE parent_payment_id IS NOT NULL LIMIT 1)",
                &[],
            )
            .await
            .map(|row| row.get(0))
            .unwrap_or(false);

        if !has_related {
            return Ok(HashMap::new());
        }

        // Build the IN clause with placeholders
        let placeholders: Vec<String> = parent_payment_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();
        let in_clause = placeholders.join(", ");

        let query = format!(
            "{SELECT_PAYMENT_SQL} WHERE pm.parent_payment_id IN ({in_clause}) ORDER BY p.timestamp ASC"
        );

        let params: Vec<&(dyn ToSql + Sync)> = parent_payment_ids
            .iter()
            .map(|id| id as &(dyn ToSql + Sync))
            .collect();

        let rows = client.query(&query, &params).await?;

        let mut result: HashMap<String, Vec<Payment>> = HashMap::new();
        for row in rows {
            let payment = map_payment(&row)?;
            let parent_payment_id: String = row.get(27);
            result.entry(parent_payment_id).or_default().push(payment);
        }

        Ok(result)
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        client
            .execute(
                "INSERT INTO unclaimed_deposits (txid, vout, amount_sats)
                 VALUES ($1, $2, $3)
                 ON CONFLICT(txid, vout) DO NOTHING",
                &[&txid, &i32::try_from(vout)?, &i64::try_from(amount_sats)?],
            )
            .await?;
        Ok(())
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        client
            .execute(
                "DELETE FROM unclaimed_deposits WHERE txid = $1 AND vout = $2",
                &[&txid, &i32::try_from(vout)?],
            )
            .await?;
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let rows = client
            .query(
                "SELECT txid, vout, amount_sats, claim_error, refund_tx, refund_tx_id FROM unclaimed_deposits",
                &[],
            )
            .await?;

        let mut deposits = Vec::new();
        for row in rows {
            let claim_error_json: Option<serde_json::Value> = row.get(3);
            let claim_error: Option<DepositClaimError> = from_json_opt(claim_error_json)?;

            deposits.push(DepositInfo {
                txid: row.get(0),
                vout: u32::try_from(row.get::<_, i32>(1))?,
                amount_sats: row
                    .get::<_, Option<i64>>(2)
                    .map(u64::try_from)
                    .transpose()?
                    .unwrap_or(0),
                claim_error,
                refund_tx: row.get(4),
                refund_tx_id: row.get(5),
            });
        }
        Ok(deposits)
    }

    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        match payload {
            UpdateDepositPayload::ClaimError { error } => {
                let error_json = serde_json::to_value(&error)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                client
                    .execute(
                        "UPDATE unclaimed_deposits SET claim_error = $1 WHERE txid = $2 AND vout = $3",
                        &[&error_json, &txid, &i32::try_from(vout)?],
                    )
                    .await?;
            }
            UpdateDepositPayload::Refund {
                refund_txid,
                refund_tx,
            } => {
                client
                    .execute(
                        "UPDATE unclaimed_deposits SET refund_tx = $1, refund_tx_id = $2 WHERE txid = $3 AND vout = $4",
                        &[&refund_tx, &refund_txid, &txid, &i32::try_from(vout)?],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn set_lnurl_metadata(
        &self,
        metadata: Vec<SetLnurlMetadataItem>,
    ) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        for m in metadata {
            client
                .execute(
                    "INSERT INTO lnurl_receive_metadata (payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT(payment_hash) DO UPDATE SET
                        nostr_zap_request = EXCLUDED.nostr_zap_request,
                        nostr_zap_receipt = EXCLUDED.nostr_zap_receipt,
                        sender_comment = EXCLUDED.sender_comment",
                    &[&m.payment_hash, &m.nostr_zap_request, &m.nostr_zap_receipt, &m.sender_comment],
                )
                .await?;
        }
        Ok(())
    }

    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, StorageError> {
        let mut client = self.pool.get().await.map_err(map_pool_error)?;

        let tx = client
            .transaction()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        // This revision is a local queue id for pending rows, not a server revision.
        let local_revision: i64 = tx
            .query_one(
                "SELECT COALESCE(MAX(revision), 0) + 1 FROM sync_outgoing",
                &[],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?
            .get(0);

        let updated_fields_json = serde_json::to_value(&record.updated_fields)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.execute(
            "INSERT INTO sync_outgoing (record_type, data_id, schema_version, commit_time, updated_fields_json, revision)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &record.id.r#type,
                &record.id.data_id,
                &record.schema_version,
                &commit_time,
                &updated_fields_json,
                &local_revision,
            ],
        )
        .await
        .map_err(|e| StorageError::Connection(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        Ok(u64::try_from(local_revision)?)
    }

    async fn complete_outgoing_sync(
        &self,
        record: Record,
        local_revision: u64,
    ) -> Result<(), StorageError> {
        let mut client = self.pool.get().await.map_err(map_pool_error)?;

        let tx = client
            .transaction()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let rows_deleted = tx
            .execute(
                "DELETE FROM sync_outgoing WHERE record_type = $1 AND data_id = $2 AND revision = $3",
                &[
                    &record.id.r#type,
                    &record.id.data_id,
                    &i64::try_from(local_revision)?,
                ],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        if rows_deleted == 0 {
            warn!(
                "complete_outgoing_sync: DELETE from sync_outgoing matched 0 rows \
                 (type={}, data_id={}, revision={})",
                record.id.r#type, record.id.data_id, local_revision
            );
        }

        let data_json = serde_json::to_value(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.execute(
            "INSERT INTO sync_state (record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(record_type, data_id) DO UPDATE SET
                    schema_version = EXCLUDED.schema_version,
                    commit_time = EXCLUDED.commit_time,
                    data = EXCLUDED.data,
                    revision = EXCLUDED.revision",
            &[
                &record.id.r#type,
                &record.id.data_id,
                &record.schema_version,
                &commit_time,
                &data_json,
                &i64::try_from(record.revision)?,
            ],
        )
        .await
        .map_err(|e| StorageError::Connection(e.to_string()))?;

        tx.execute(
            "UPDATE sync_revision SET revision = GREATEST(revision, $1)",
            &[&i64::try_from(record.revision)?],
        )
        .await
        .map_err(|e| StorageError::Connection(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        Ok(())
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let rows = client
            .query(
                "SELECT o.record_type, o.data_id, o.schema_version, o.commit_time, o.updated_fields_json, o.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM sync_outgoing o
                 LEFT JOIN sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id
                 ORDER BY o.revision ASC
                 LIMIT $1",
                &[&i64::from(limit)],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let parent = if let Some(existing_data) = row.get::<_, Option<serde_json::Value>>(8) {
                Some(Record {
                    id: RecordId::new(row.get(0), row.get(1)),
                    schema_version: row.get(6),
                    revision: u64::try_from(row.get::<_, i64>(9))?,
                    data: serde_json::from_value(existing_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                })
            } else {
                None
            };
            let change = RecordChange {
                id: RecordId::new(row.get(0), row.get(1)),
                schema_version: row.get(2),
                updated_fields: serde_json::from_value(row.get::<_, serde_json::Value>(4))
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                local_revision: u64::try_from(row.get::<_, i64>(5))?,
            };
            results.push(OutgoingChange { change, parent });
        }

        Ok(results)
    }

    async fn get_last_revision(&self) -> Result<u64, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let revision: i64 = client
            .query_one("SELECT revision FROM sync_revision", &[])
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?
            .get(0);

        Ok(u64::try_from(revision)?)
    }

    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError> {
        if records.is_empty() {
            return Ok(());
        }

        let client = self.pool.get().await.map_err(map_pool_error)?;
        let commit_time = chrono::Utc::now().timestamp();

        for record in records {
            let data_json = serde_json::to_value(&record.data)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            client
                .execute(
                    "INSERT INTO sync_incoming (record_type, data_id, schema_version, commit_time, data, revision)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT(record_type, data_id, revision) DO UPDATE SET
                        schema_version = EXCLUDED.schema_version,
                        commit_time = EXCLUDED.commit_time,
                        data = EXCLUDED.data",
                    &[
                        &record.id.r#type,
                        &record.id.data_id,
                        &record.schema_version,
                        &commit_time,
                        &data_json,
                        &i64::try_from(record.revision)?,
                    ],
                )
                .await
                .map_err(|e| StorageError::Connection(e.to_string()))?;
        }

        Ok(())
    }

    async fn delete_incoming_record(&self, record: Record) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        client
            .execute(
                "DELETE FROM sync_incoming WHERE record_type = $1 AND data_id = $2 AND revision = $3",
                &[
                    &record.id.r#type,
                    &record.id.data_id,
                    &i64::try_from(record.revision)?,
                ],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        Ok(())
    }

    async fn get_incoming_records(&self, limit: u32) -> Result<Vec<IncomingChange>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let rows = client
            .query(
                "SELECT i.record_type, i.data_id, i.schema_version, i.data, i.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM sync_incoming i
                 LEFT JOIN sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id
                 ORDER BY i.revision ASC
                 LIMIT $1",
                &[&i64::from(limit)],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let old_state = if let Some(existing_data) = row.get::<_, Option<serde_json::Value>>(7)
            {
                Some(Record {
                    id: RecordId::new(row.get(0), row.get(1)),
                    schema_version: row.get(5),
                    revision: u64::try_from(row.get::<_, i64>(8))?,
                    data: serde_json::from_value(existing_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                })
            } else {
                None
            };
            let new_state = Record {
                id: RecordId::new(row.get(0), row.get(1)),
                schema_version: row.get(2),
                data: serde_json::from_value(row.get::<_, serde_json::Value>(3))
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                revision: u64::try_from(row.get::<_, i64>(4))?,
            };
            results.push(IncomingChange {
                new_state,
                old_state,
            });
        }

        Ok(results)
    }

    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let row = client
            .query_opt(
                "SELECT o.record_type, o.data_id, o.schema_version, o.commit_time, o.updated_fields_json, o.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM sync_outgoing o
                 LEFT JOIN sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id
                 ORDER BY o.revision DESC
                 LIMIT 1",
                &[],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        if let Some(row) = row {
            let parent = if let Some(existing_data) = row.get::<_, Option<serde_json::Value>>(8) {
                Some(Record {
                    id: RecordId::new(row.get(0), row.get(1)),
                    schema_version: row.get(6),
                    revision: u64::try_from(row.get::<_, i64>(9))?,
                    data: serde_json::from_value(existing_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                })
            } else {
                None
            };
            let change = RecordChange {
                id: RecordId::new(row.get(0), row.get(1)),
                schema_version: row.get(2),
                updated_fields: serde_json::from_value(row.get::<_, serde_json::Value>(4))
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                local_revision: u64::try_from(row.get::<_, i64>(5))?,
            };
            return Ok(Some(OutgoingChange { change, parent }));
        }

        Ok(None)
    }

    async fn update_record_from_incoming(&self, record: Record) -> Result<(), StorageError> {
        let mut client = self.pool.get().await.map_err(map_pool_error)?;

        let tx = client
            .transaction()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let data_json = serde_json::to_value(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.execute(
            "INSERT INTO sync_state (record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(record_type, data_id) DO UPDATE SET
                    schema_version = EXCLUDED.schema_version,
                    commit_time = EXCLUDED.commit_time,
                    data = EXCLUDED.data,
                    revision = EXCLUDED.revision",
            &[
                &record.id.r#type,
                &record.id.data_id,
                &record.schema_version,
                &commit_time,
                &data_json,
                &i64::try_from(record.revision)?,
            ],
        )
        .await
        .map_err(|e| StorageError::Connection(e.to_string()))?;

        tx.execute(
            "UPDATE sync_revision SET revision = GREATEST(revision, $1)",
            &[&i64::try_from(record.revision)?],
        )
        .await
        .map_err(|e| StorageError::Connection(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        Ok(())
    }
}

/// Base query for payment lookups.
/// Column indices 0-26 are used by `map_payment`, index 27 (`parent_payment_id`) is only used by `get_payments_by_parent_ids`.
const SELECT_PAYMENT_SQL: &str = "
    SELECT p.id,
           p.payment_type,
           p.status,
           p.amount,
           p.fees,
           p.timestamp,
           p.method,
           p.withdraw_tx_id,
           p.deposit_tx_id,
           p.spark,
           l.invoice AS lightning_invoice,
           l.payment_hash AS lightning_payment_hash,
           l.destination_pubkey AS lightning_destination_pubkey,
           COALESCE(l.description, pm.lnurl_description) AS lightning_description,
           l.preimage AS lightning_preimage,
           pm.lnurl_pay_info,
           pm.lnurl_withdraw_info,
           pm.conversion_info,
           t.metadata AS token_metadata,
           t.tx_hash AS token_tx_hash,
           t.tx_type AS token_tx_type,
           t.invoice_details AS token_invoice_details,
           s.invoice_details AS spark_invoice_details,
           s.htlc_details AS spark_htlc_details,
           lrm.nostr_zap_request AS lnurl_nostr_zap_request,
           lrm.nostr_zap_receipt AS lnurl_nostr_zap_receipt,
           lrm.sender_comment AS lnurl_sender_comment,
           pm.parent_payment_id
      FROM payments p
      LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
      LEFT JOIN payment_details_token t ON p.id = t.payment_id
      LEFT JOIN payment_details_spark s ON p.id = s.payment_id
      LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
      LEFT JOIN lnurl_receive_metadata lrm ON l.payment_hash = lrm.payment_hash";

#[allow(clippy::too_many_lines)]
fn map_payment(row: &Row) -> Result<Payment, StorageError> {
    let withdraw_tx_id: Option<String> = row.get(7);
    let deposit_tx_id: Option<String> = row.get(8);
    let spark: Option<bool> = row.get(9);
    let lightning_invoice: Option<String> = row.get(10);
    let token_metadata: Option<serde_json::Value> = row.get(18);

    let details = match (
        lightning_invoice,
        withdraw_tx_id,
        deposit_tx_id,
        spark,
        token_metadata,
    ) {
        (Some(invoice), _, _, _, _) => {
            let payment_hash: String = row.get(11);
            let destination_pubkey: String = row.get(12);
            let description: Option<String> = row.get(13);
            let preimage: Option<String> = row.get(14);
            let lnurl_pay_info_json: Option<serde_json::Value> = row.get(15);
            let lnurl_withdraw_info_json: Option<serde_json::Value> = row.get(16);
            let lnurl_nostr_zap_request: Option<String> = row.get(24);
            let lnurl_nostr_zap_receipt: Option<String> = row.get(25);
            let lnurl_sender_comment: Option<String> = row.get(26);

            let lnurl_pay_info: Option<LnurlPayInfo> = from_json_opt(lnurl_pay_info_json)?;
            let lnurl_withdraw_info: Option<LnurlWithdrawInfo> =
                from_json_opt(lnurl_withdraw_info_json)?;

            let lnurl_receive_metadata =
                if lnurl_nostr_zap_request.is_some() || lnurl_sender_comment.is_some() {
                    Some(LnurlReceiveMetadata {
                        nostr_zap_request: lnurl_nostr_zap_request,
                        nostr_zap_receipt: lnurl_nostr_zap_receipt,
                        sender_comment: lnurl_sender_comment,
                    })
                } else {
                    None
                };
            Some(PaymentDetails::Lightning {
                invoice,
                payment_hash,
                destination_pubkey,
                description,
                preimage,
                lnurl_pay_info,
                lnurl_withdraw_info,
                lnurl_receive_metadata,
            })
        }
        (_, Some(tx_id), _, _, _) => Some(PaymentDetails::Withdraw { tx_id }),
        (_, _, Some(tx_id), _, _) => Some(PaymentDetails::Deposit { tx_id }),
        (_, _, _, Some(_), _) => {
            let invoice_details_json: Option<serde_json::Value> = row.get(22);
            let invoice_details = from_json_opt(invoice_details_json)?;
            let htlc_details_json: Option<serde_json::Value> = row.get(23);
            let htlc_details = from_json_opt(htlc_details_json)?;
            let conversion_info_json: Option<serde_json::Value> = row.get(17);
            let conversion_info: Option<ConversionInfo> = from_json_opt(conversion_info_json)?;
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                conversion_info,
            })
        }
        (_, _, _, _, Some(metadata)) => {
            let tx_type_str: String = row.get(20);
            let tx_type = tx_type_str
                .parse()
                .map_err(|e: String| StorageError::Serialization(e))?;
            let invoice_details_json: Option<serde_json::Value> = row.get(21);
            let invoice_details = from_json_opt(invoice_details_json)?;
            let conversion_info_json: Option<serde_json::Value> = row.get(17);
            let conversion_info: Option<ConversionInfo> = from_json_opt(conversion_info_json)?;
            Some(PaymentDetails::Token {
                metadata: serde_json::from_value(metadata)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                tx_hash: row.get(19),
                tx_type,
                invoice_details,
                conversion_info,
            })
        }
        _ => None,
    };

    let payment_type_str: String = row.get(1);
    let status_str: String = row.get(2);
    let amount_str: String = row.get(3);
    let fees_str: String = row.get(4);
    let method_str: Option<String> = row.get(6);

    Ok(Payment {
        id: row.get(0),
        payment_type: payment_type_str
            .parse()
            .map_err(|e: String| StorageError::Serialization(e))?,
        status: status_str
            .parse()
            .map_err(|e: String| StorageError::Serialization(e))?,
        amount: amount_str
            .parse()
            .map_err(|_| StorageError::Serialization("invalid amount".to_string()))?,
        fees: fees_str
            .parse()
            .map_err(|_| StorageError::Serialization("invalid fees".to_string()))?,
        timestamp: u64::try_from(row.get::<_, i64>(5))?,
        details,
        method: method_str.map_or(PaymentMethod::Lightning, |s| {
            s.trim_matches('"')
                .to_lowercase()
                .parse()
                .unwrap_or(PaymentMethod::Lightning)
        }),
        conversion_details: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Helper struct that holds the container and storage together.
    /// The container must be kept alive for the duration of the test.
    struct PostgresTestFixture {
        storage: PostgresStorage,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    impl PostgresTestFixture {
        async fn new() -> Self {
            // Start a PostgreSQL container using testcontainers
            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start PostgreSQL container");

            // Get the host port that maps to PostgreSQL's port 5432
            let host_port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get host port");

            // Build connection string for the container
            let connection_string = format!(
                "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
            );

            let storage =
                PostgresStorage::new(PostgresStorageConfig::with_defaults(connection_string))
                    .await
                    .expect("Failed to create PostgresStorage");

            Self { storage, container }
        }
    }

    #[tokio::test]
    async fn test_postgres_storage() {
        let fixture = PostgresTestFixture::new().await;
        Box::pin(crate::persist::tests::test_storage(Box::new(
            fixture.storage,
        )))
        .await;
    }

    #[tokio::test]
    async fn test_unclaimed_deposits_crud() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_unclaimed_deposits_crud(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_deposit_refunds() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_deposit_refunds(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_type_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_payment_type_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_status_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_payment_status_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_asset_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_asset_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_timestamp_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_timestamp_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_spark_htlc_status_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_spark_htlc_status_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_conversion_refund_needed_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_conversion_refund_needed_filtering(Box::new(fixture.storage))
            .await;
    }

    #[tokio::test]
    async fn test_token_transaction_type_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_token_transaction_type_filtering(Box::new(fixture.storage))
            .await;
    }

    #[tokio::test]
    async fn test_combined_filters() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_combined_filters(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_sort_order() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_sort_order(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_metadata() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_payment_metadata(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_details_update_persistence() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_payment_details_update_persistence(Box::new(fixture.storage))
            .await;
    }

    #[tokio::test]
    async fn test_payment_metadata_merge() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_payment_metadata_merge(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_sync_storage() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_sync_storage(Box::new(fixture.storage)).await;
    }

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
