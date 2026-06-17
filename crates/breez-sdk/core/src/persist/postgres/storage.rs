//! PostgreSQL-backed implementation of the `Storage` trait.
//!
//! This module provides the main SDK storage implementation backed by `PostgreSQL`,
//! suitable for server-side or multi-instance deployments.

use std::collections::HashMap;

use bitcoin::hashes::{Hash, HashEngine, sha256};
use macros::async_trait;
use spark_postgres::deadpool_postgres;
use spark_postgres::tokio_postgres;

use deadpool_postgres::Pool;
use tokio_postgres::{Row, Transaction, types::ToSql};
use tracing::warn;

use crate::{
    AssetFilter, Contact, ConversionDetails, ConversionInfo, ConversionStatus, DepositInfo,
    ListContactsRequest, LnurlPayInfo, LnurlReceiveMetadata, LnurlWithdrawInfo, PaymentDetails,
    PaymentMethod, PaymentStatus, SparkHtlcDetails, SparkHtlcStatus,
    error::DepositClaimError,
    persist::{
        Payment, PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError,
        StorageListPaymentsRequest, StoragePaymentDetailsFilter, UpdateDepositPayload,
        parse_payment_status,
    },
    sync_storage::{
        IncomingChange, OutgoingChange, Record, RecordChange, RecordId, UnversionedRecordChange,
    },
};

#[cfg(test)]
use super::base::{PostgresStorageConfig, create_pool};
use super::base::{SchemaRenames, map_db_error, map_pool_error, run_migrations};

/// Name of the schema migrations table for `PostgresStorage`.
const MIGRATIONS_TABLE: &str = "brz_schema_migrations";

/// Pre-prefix rename map for upgrading core persist deployments.
const SCHEMA_RENAMES: SchemaRenames<'static> = SchemaRenames {
    old_migrations_table: "schema_migrations",
    new_migrations_table: MIGRATIONS_TABLE,
    tables: &[
        ("payments", "brz_payments"),
        ("settings", "brz_settings"),
        ("unclaimed_deposits", "brz_unclaimed_deposits"),
        ("payment_metadata", "brz_payment_metadata"),
        ("payment_details_lightning", "brz_payment_details_lightning"),
        ("payment_details_token", "brz_payment_details_token"),
        ("payment_details_spark", "brz_payment_details_spark"),
        ("lnurl_receive_metadata", "brz_lnurl_receive_metadata"),
        ("sync_revision", "brz_sync_revision"),
        ("sync_outgoing", "brz_sync_outgoing"),
        ("sync_state", "brz_sync_state"),
        ("sync_incoming", "brz_sync_incoming"),
        ("contacts", "brz_contacts"),
    ],
    indexes: &[
        // Post-multi-tenant indexes (current state on version >= 16 DBs).
        (
            "idx_payments_user_timestamp",
            "brz_idx_payments_user_timestamp",
        ),
        (
            "idx_payments_user_payment_type",
            "brz_idx_payments_user_payment_type",
        ),
        ("idx_payments_user_status", "brz_idx_payments_user_status"),
        (
            "idx_payment_metadata_user_parent",
            "brz_idx_payment_metadata_user_parent",
        ),
        (
            "idx_payment_details_lightning_user_invoice",
            "brz_idx_payment_details_lightning_user_invoice",
        ),
        (
            "idx_payment_details_lightning_user_payment_hash",
            "brz_idx_payment_details_lightning_user_payment_hash",
        ),
        (
            "idx_sync_outgoing_user_record_type_data_id",
            "brz_idx_sync_outgoing_user_record_type_data_id",
        ),
        (
            "idx_sync_incoming_user_revision",
            "brz_idx_sync_incoming_user_revision",
        ),
        // Pre-multi-tenant indexes (still present on version < 16 DBs).
        // The multi-tenant migration drops these via `DROP INDEX IF EXISTS
        // brz_idx_*`, so the rename moves them under the prefix first.
        ("idx_payments_timestamp", "brz_idx_payments_timestamp"),
        ("idx_payments_payment_type", "brz_idx_payments_payment_type"),
        ("idx_payments_status", "brz_idx_payments_status"),
        (
            "idx_payment_metadata_parent",
            "brz_idx_payment_metadata_parent",
        ),
        (
            "idx_payment_details_lightning_invoice",
            "brz_idx_payment_details_lightning_invoice",
        ),
        (
            "idx_payment_details_lightning_payment_hash",
            "brz_idx_payment_details_lightning_payment_hash",
        ),
        (
            "idx_sync_outgoing_data_id_record_type",
            "brz_idx_sync_outgoing_data_id_record_type",
        ),
        (
            "idx_sync_incoming_revision",
            "brz_idx_sync_incoming_revision",
        ),
    ],
    constraints: &[
        ("brz_payments", "payments_pkey", "brz_payments_pkey"),
        ("brz_settings", "settings_pkey", "brz_settings_pkey"),
        (
            "brz_unclaimed_deposits",
            "unclaimed_deposits_pkey",
            "brz_unclaimed_deposits_pkey",
        ),
        (
            "brz_payment_metadata",
            "payment_metadata_pkey",
            "brz_payment_metadata_pkey",
        ),
        (
            "brz_payment_details_lightning",
            "payment_details_lightning_pkey",
            "brz_payment_details_lightning_pkey",
        ),
        (
            "brz_payment_details_token",
            "payment_details_token_pkey",
            "brz_payment_details_token_pkey",
        ),
        (
            "brz_payment_details_spark",
            "payment_details_spark_pkey",
            "brz_payment_details_spark_pkey",
        ),
        (
            "brz_lnurl_receive_metadata",
            "lnurl_receive_metadata_pkey",
            "brz_lnurl_receive_metadata_pkey",
        ),
        (
            "brz_sync_revision",
            "sync_revision_pkey",
            "brz_sync_revision_pkey",
        ),
        ("brz_sync_state", "sync_state_pkey", "brz_sync_state_pkey"),
        (
            "brz_sync_incoming",
            "sync_incoming_pkey",
            "brz_sync_incoming_pkey",
        ),
        ("brz_contacts", "contacts_pkey", "brz_contacts_pkey"),
    ],
};

/// PostgreSQL-based storage implementation using connection pooling.
///
/// Each instance is scoped to a single tenant identity (a 33-byte secp256k1
/// compressed public key). All reads and writes are filtered by `user_id` so
/// that multiple instances with distinct identities can share one Postgres DB
/// without seeing each other's data.
pub(crate) struct PostgresStorage {
    pool: Pool,
    /// Tenant identity: 33-byte compressed secp256k1 pubkey. Stored as raw
    /// bytes for direct binding to BYTEA columns.
    identity: Vec<u8>,
}

impl PostgresStorage {
    /// Creates a new `PostgresStorage` with a connection pool.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the `PostgreSQL` connection pool
    /// * `identity` - 33-byte compressed secp256k1 public key uniquely identifying this tenant
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
    #[cfg(test)]
    pub async fn new(config: PostgresStorageConfig, identity: &[u8]) -> Result<Self, StorageError> {
        let run_migration = config.run_migration;
        let pool = create_pool(&config)?;
        Self::new_with_pool(pool, identity, run_migration).await
    }

    /// Creates a new `PostgresStorage` using an existing connection pool.
    ///
    /// This allows sharing a single pool across multiple store implementations.
    /// Each `PostgresStorage` is scoped to a single tenant `identity`. When
    /// `run_migration` is `false`, initialization trusts the existing schema
    /// and skips SDK storage migrations entirely.
    pub async fn new_with_pool(
        pool: Pool,
        identity: &[u8],
        run_migration: bool,
    ) -> Result<Self, StorageError> {
        let storage = Self {
            pool,
            identity: identity.to_vec(),
        };
        if run_migration {
            storage.migrate().await?;
        }
        Ok(storage)
    }

    async fn migrate(&self) -> Result<(), StorageError> {
        run_migrations(
            &self.pool,
            MIGRATIONS_TABLE,
            &Self::migrations(&self.identity),
            Some(&SCHEMA_RENAMES),
        )
        .await
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn migrations(identity: &[u8]) -> Vec<Vec<String>> {
        vec![
            // Migration 1: Core tables
            vec![
                "CREATE TABLE IF NOT EXISTS brz_payments (
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
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_unclaimed_deposits (
                    txid TEXT NOT NULL,
                    vout INTEGER NOT NULL,
                    amount_sats BIGINT,
                    claim_error JSONB,
                    refund_tx TEXT,
                    refund_tx_id TEXT,
                    PRIMARY KEY (txid, vout)
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_payment_metadata (
                    payment_id TEXT PRIMARY KEY,
                    parent_payment_id TEXT,
                    lnurl_pay_info JSONB,
                    lnurl_withdraw_info JSONB,
                    lnurl_description TEXT,
                    conversion_info JSONB
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_payment_details_lightning (
                    payment_id TEXT PRIMARY KEY,
                    invoice TEXT NOT NULL,
                    payment_hash TEXT NOT NULL,
                    destination_pubkey TEXT NOT NULL,
                    description TEXT,
                    preimage TEXT
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_payment_details_token (
                    payment_id TEXT PRIMARY KEY,
                    metadata JSONB NOT NULL,
                    tx_hash TEXT NOT NULL,
                    invoice_details JSONB
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_payment_details_spark (
                    payment_id TEXT PRIMARY KEY,
                    invoice_details JSONB,
                    htlc_details JSONB
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_lnurl_receive_metadata (
                    payment_hash TEXT PRIMARY KEY,
                    nostr_zap_request TEXT,
                    nostr_zap_receipt TEXT,
                    sender_comment TEXT
                )".to_string(),
            ],
            // Migration 2: Sync tables
            vec![
                // brz_sync_revision: tracks the last committed revision (from server-acknowledged
                // or server-received records). Does NOT include pending outgoing queue ids.
                // brz_sync_outgoing.revision stores a local queue id for ordering/de-duplication only.
                "CREATE TABLE IF NOT EXISTS brz_sync_revision (
                    id INTEGER PRIMARY KEY DEFAULT 1,
                    revision BIGINT NOT NULL DEFAULT 0,
                    CHECK (id = 1)
                )".to_string(),
                "INSERT INTO brz_sync_revision (id, revision) VALUES (1, 0) ON CONFLICT (id) DO NOTHING".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_sync_outgoing (
                    record_type TEXT NOT NULL,
                    data_id TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    commit_time BIGINT NOT NULL,
                    updated_fields_json JSONB NOT NULL,
                    revision BIGINT NOT NULL
                )".to_string(),
                "CREATE INDEX IF NOT EXISTS brz_idx_sync_outgoing_data_id_record_type ON brz_sync_outgoing(record_type, data_id)".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_sync_state (
                    record_type TEXT NOT NULL,
                    data_id TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    commit_time BIGINT NOT NULL,
                    data JSONB NOT NULL,
                    revision BIGINT NOT NULL,
                    PRIMARY KEY(record_type, data_id)
                )".to_string(),
                "CREATE TABLE IF NOT EXISTS brz_sync_incoming (
                    record_type TEXT NOT NULL,
                    data_id TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    commit_time BIGINT NOT NULL,
                    data JSONB NOT NULL,
                    revision BIGINT NOT NULL,
                    PRIMARY KEY(record_type, data_id, revision)
                )".to_string(),
                "CREATE INDEX IF NOT EXISTS brz_idx_sync_incoming_revision ON brz_sync_incoming(revision)".to_string(),
            ],
            // Migration 3: Indexes
            vec![
                "CREATE INDEX IF NOT EXISTS brz_idx_payments_timestamp ON brz_payments(timestamp)".to_string(),
                "CREATE INDEX IF NOT EXISTS brz_idx_payments_payment_type ON brz_payments(payment_type)".to_string(),
                "CREATE INDEX IF NOT EXISTS brz_idx_payments_status ON brz_payments(status)".to_string(),
                "CREATE INDEX IF NOT EXISTS brz_idx_payment_details_lightning_invoice ON brz_payment_details_lightning(invoice)".to_string(),
                "CREATE INDEX IF NOT EXISTS brz_idx_payment_metadata_parent ON brz_payment_metadata(parent_payment_id)".to_string(),
            ],
            // Migration 4: Add tx_type to token payments
            vec![
                "ALTER TABLE brz_payment_details_token ADD COLUMN tx_type TEXT NOT NULL DEFAULT 'transfer'".to_string(),
            ],
            // Migration 5: Clear sync tables to force re-sync
            vec![
                "DELETE FROM brz_sync_outgoing".to_string(),
                "DELETE FROM brz_sync_incoming".to_string(),
                "DELETE FROM brz_sync_state".to_string(),
                "UPDATE brz_sync_revision SET revision = 0".to_string(),
                "DELETE FROM brz_settings WHERE key = 'sync_initial_complete'".to_string(),
            ],
            // Migration 6: Add htlc_status and htlc_expiry_time to lightning payments
            vec![
                "ALTER TABLE brz_payment_details_lightning ADD COLUMN htlc_status TEXT NOT NULL DEFAULT 'WaitingForPreimage'".to_string(),
                "ALTER TABLE brz_payment_details_lightning ADD COLUMN htlc_expiry_time BIGINT NOT NULL DEFAULT 0".to_string(),
            ],
            // Migration 7: Backfill htlc_status for existing Lightning payments
            vec![
                "UPDATE brz_payment_details_lightning
                 SET htlc_status = CASE
                         WHEN (SELECT status FROM brz_payments WHERE id = payment_id) = 'completed' THEN 'PreimageShared'
                         WHEN (SELECT status FROM brz_payments WHERE id = payment_id) = 'pending' THEN 'WaitingForPreimage'
                         ELSE 'Returned'
                     END".to_string(),
                "UPDATE brz_settings
                 SET value = jsonb_set(value::jsonb, '{offset}', '0')::text
                 WHERE key = 'sync_offset' AND value IS NOT NULL".to_string(),
            ],
            // Migration 8: Add preimage column for LUD-21 and NIP-57 support
            vec![
                "ALTER TABLE brz_lnurl_receive_metadata ADD COLUMN IF NOT EXISTS preimage TEXT".to_string(),
                // Clear the lnurl_metadata_updated_after setting to force re-sync
                // This ensures clients get the new preimage field from the server
                "DELETE FROM brz_settings WHERE key = 'lnurl_metadata_updated_after'".to_string(),
            ],
            // Migration 9: Clear cached lightning address - schema changed from string to LnurlInfo struct
            vec![
                "DELETE FROM brz_settings WHERE key = 'lightning_address'".to_string(),
            ],
            // Migration 10: Add index on payment_hash for JOIN with brz_lnurl_receive_metadata
            vec![
                "CREATE INDEX IF NOT EXISTS brz_idx_payment_details_lightning_payment_hash ON brz_payment_details_lightning(payment_hash)".to_string(),
            ],
            // Migration 11: Contacts table
            vec!["CREATE TABLE IF NOT EXISTS brz_contacts (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    payment_identifier TEXT NOT NULL,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL
                )".to_string()],
            // Migration 12: Drop preimage column from brz_lnurl_receive_metadata - no longer needed
            // since the server handles preimage tracking via webhooks.
            vec!["ALTER TABLE brz_lnurl_receive_metadata DROP COLUMN IF EXISTS preimage".to_string()],
            // Migration 13: Clear cached lightning address - format changed to CachedLightningAddress wrapper
            vec!["DELETE FROM brz_settings WHERE key = 'lightning_address'".to_string()],
            // Migration 14: Add is_mature to brz_unclaimed_deposits
            vec![
                "ALTER TABLE brz_unclaimed_deposits ADD COLUMN is_mature BOOLEAN NOT NULL DEFAULT TRUE".to_string(),
            ],
            // Migration 15: Add conversion_status to brz_payment_metadata
            vec!["ALTER TABLE brz_payment_metadata ADD COLUMN IF NOT EXISTS conversion_status TEXT".to_string()],
            // Migration 16: Multi-tenant scoping. Adds a `user_id BYTEA` column to every
            // per-user table, backfills it to the current tenant's identity (so existing
            // single-tenant deployments remain readable), sets NOT NULL, and rewrites
            // primary keys / indexes to lead with `user_id`. The literal hex of `identity`
            // is inlined into the SQL: identity bytes come from a typed secp256k1 pubkey
            // so the character set is restricted to `[0-9a-f]{66}` — no SQL-injection
            // surface even though the value is concatenated rather than parameter-bound.
            // (Migrations are run as untyped batch_execute, so parameter binding is not
            // available without restructuring the runner.)
            multi_tenant_migration(identity),
            // Migration 17: Move deposit details into their own table so vout can be
            // NOT NULL and the schema matches brz_payment_details_lightning / _token /
            // _spark. We can't safely backfill the new table from the dropped
            // deposit_tx_id column: we never stored the original SSP output_index,
            // and vout=0 is a valid output index, so defaulting would silently
            // mislabel. Drop the column and leave the brz_payments row in place.
            // The read path sees an unjoined deposit row as `details: None` until
            // the resync re-fetches the SSP user_request and the upsert inserts the
            // new details row.
            vec![
                "CREATE TABLE IF NOT EXISTS brz_payment_details_deposit (
                    user_id BYTEA NOT NULL,
                    payment_id TEXT NOT NULL,
                    tx_id TEXT NOT NULL,
                    vout BIGINT NOT NULL,
                    PRIMARY KEY (user_id, payment_id)
                 )".to_string(),
                "ALTER TABLE brz_payments DROP COLUMN IF EXISTS deposit_tx_id".to_string(),
                "UPDATE brz_settings
                 SET value = jsonb_set(value::jsonb, '{offset}', '0')::text
                 WHERE key = 'sync_offset' AND value IS NOT NULL".to_string(),
            ],
            // Migration 18: Backfill type discriminator on conversion_info for
            // the ConversionInfo enum refactor. All existing rows are AMM.
            vec!["UPDATE brz_payment_metadata
               SET conversion_info = conversion_info::jsonb || '{\"type\": \"amm\"}'::jsonb
               WHERE conversion_info IS NOT NULL
                 AND (conversion_info::jsonb->>'type') IS NULL".to_string()],
        ]
    }
}

/// Builds the multi-tenant scoping migration. The `identity` is a 33-byte
/// compressed secp256k1 pubkey; it's hex-encoded and inlined as a BYTEA literal
/// so it can be parameter-free SQL (the migration runner uses `batch_execute`).
fn multi_tenant_migration(identity: &[u8]) -> Vec<String> {
    let id_hex = hex::encode(identity);
    let id_lit = format!("'\\x{id_hex}'::bytea");

    let scope_table = |table: &str, pk_cols: &str| -> Vec<String> {
        vec![
            format!("ALTER TABLE {table} ADD COLUMN user_id BYTEA"),
            format!("UPDATE {table} SET user_id = {id_lit}"),
            format!(
                "ALTER TABLE {table} \
                 ALTER COLUMN user_id SET NOT NULL, \
                 DROP CONSTRAINT IF EXISTS {table}_pkey, \
                 ADD PRIMARY KEY (user_id, {pk_cols})"
            ),
        ]
    };

    let mut stmts = Vec::new();

    stmts.extend(scope_table("brz_payments", "id"));
    // Per-user index rewrite for brz_payments
    stmts.push("DROP INDEX IF EXISTS brz_idx_payments_timestamp".to_string());
    stmts.push("DROP INDEX IF EXISTS brz_idx_payments_payment_type".to_string());
    stmts.push("DROP INDEX IF EXISTS brz_idx_payments_status".to_string());
    stmts.push(
        "CREATE INDEX brz_idx_payments_user_timestamp ON brz_payments(user_id, timestamp)"
            .to_string(),
    );
    stmts.push(
        "CREATE INDEX brz_idx_payments_user_payment_type ON brz_payments(user_id, payment_type)"
            .to_string(),
    );
    stmts.push(
        "CREATE INDEX brz_idx_payments_user_status ON brz_payments(user_id, status)".to_string(),
    );

    stmts.extend(scope_table("brz_payment_metadata", "payment_id"));
    stmts.push("DROP INDEX IF EXISTS brz_idx_payment_metadata_parent".to_string());
    stmts.push(
        "CREATE INDEX brz_idx_payment_metadata_user_parent \
         ON brz_payment_metadata(user_id, parent_payment_id)"
            .to_string(),
    );

    stmts.extend(scope_table("brz_payment_details_lightning", "payment_id"));
    stmts.push("DROP INDEX IF EXISTS brz_idx_payment_details_lightning_invoice".to_string());
    stmts.push("DROP INDEX IF EXISTS brz_idx_payment_details_lightning_payment_hash".to_string());
    stmts.push(
        "CREATE INDEX brz_idx_payment_details_lightning_user_invoice \
         ON brz_payment_details_lightning(user_id, invoice)"
            .to_string(),
    );
    stmts.push(
        "CREATE INDEX brz_idx_payment_details_lightning_user_payment_hash \
         ON brz_payment_details_lightning(user_id, payment_hash)"
            .to_string(),
    );

    stmts.extend(scope_table("brz_payment_details_token", "payment_id"));
    stmts.extend(scope_table("brz_payment_details_spark", "payment_id"));
    stmts.extend(scope_table("brz_lnurl_receive_metadata", "payment_hash"));
    stmts.extend(scope_table("brz_unclaimed_deposits", "txid, vout"));
    stmts.extend(scope_table("brz_contacts", "id"));
    stmts.extend(scope_table("brz_settings", "key"));

    // brz_sync_revision was a single-row table (PK id=1, CHECK id=1). Drop the id column
    // (CASCADE clears the PK and the CHECK), then re-key by user_id so every tenant
    // has its own revision counter.
    stmts.push("ALTER TABLE brz_sync_revision DROP COLUMN id CASCADE".to_string());
    stmts.push("ALTER TABLE brz_sync_revision ADD COLUMN user_id BYTEA".to_string());
    stmts.push(format!("UPDATE brz_sync_revision SET user_id = {id_lit}"));
    stmts.push(
        "ALTER TABLE brz_sync_revision \
         ALTER COLUMN user_id SET NOT NULL, \
         ADD PRIMARY KEY (user_id)"
            .to_string(),
    );

    // brz_sync_outgoing has no PK, only an index — just add user_id and rewrite the index.
    stmts.push("ALTER TABLE brz_sync_outgoing ADD COLUMN user_id BYTEA".to_string());
    stmts.push(format!("UPDATE brz_sync_outgoing SET user_id = {id_lit}"));
    stmts.push("ALTER TABLE brz_sync_outgoing ALTER COLUMN user_id SET NOT NULL".to_string());
    stmts.push("DROP INDEX IF EXISTS brz_idx_sync_outgoing_data_id_record_type".to_string());
    stmts.push(
        "CREATE INDEX brz_idx_sync_outgoing_user_record_type_data_id \
         ON brz_sync_outgoing(user_id, record_type, data_id)"
            .to_string(),
    );

    stmts.extend(scope_table("brz_sync_state", "record_type, data_id"));

    stmts.extend(scope_table(
        "brz_sync_incoming",
        "record_type, data_id, revision",
    ));
    stmts.push("DROP INDEX IF EXISTS brz_idx_sync_incoming_revision".to_string());
    stmts.push(
        "CREATE INDEX brz_idx_sync_incoming_user_revision ON brz_sync_incoming(user_id, revision)"
            .to_string(),
    );

    stmts
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

impl PostgresStorage {
    fn payment_update_lock_key(identity: &[u8], payment_id: &str) -> i64 {
        let mut engine = sha256::Hash::engine();
        engine.input(b"brz_payment_update");
        engine.input(identity);
        engine.input(payment_id.as_bytes());
        let digest = sha256::Hash::from_engine(engine);
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&digest.as_byte_array()[..8]);
        i64::from_be_bytes(buf)
    }

    async fn get_payment_status_in_tx(
        tx: &Transaction<'_>,
        identity: &[u8],
        payment_id: &str,
    ) -> Result<Option<PaymentStatus>, StorageError> {
        let row = tx
            .query_opt(
                "SELECT status FROM brz_payments WHERE user_id = $1 AND id = $2 FOR UPDATE",
                &[&identity, &payment_id],
            )
            .await
            .map_err(map_db_error)?;

        row.map(|row| {
            let status: String = row.get(0);
            parse_payment_status(&status)
        })
        .transpose()
    }

    #[allow(clippy::too_many_lines)]
    async fn insert_payment_in_tx(
        tx: &Transaction<'_>,
        identity: &[u8],
        payment: Payment,
    ) -> Result<(), StorageError> {
        // Compute detail columns for the main payments row
        let (withdraw_tx_id, spark): (Option<&str>, Option<bool>) = match &payment.details {
            Some(PaymentDetails::Withdraw { tx_id }) => (Some(tx_id.as_str()), None),
            Some(PaymentDetails::Spark { .. }) => (None, Some(true)),
            _ => (None, None),
        };

        // Insert or update main payment record (including detail columns atomically)
        tx.execute(
            "INSERT INTO brz_payments (user_id, id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, spark)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(user_id, id) DO UPDATE SET
                    payment_type = EXCLUDED.payment_type,
                    status = EXCLUDED.status,
                    amount = EXCLUDED.amount,
                    fees = EXCLUDED.fees,
                    timestamp = EXCLUDED.timestamp,
                    method = EXCLUDED.method,
                    withdraw_tx_id = EXCLUDED.withdraw_tx_id,
                    spark = EXCLUDED.spark",
            &[
                &identity,
                &payment.id,
                &payment.payment_type.to_string(),
                &payment.status.to_string(),
                &payment.amount.to_string(),
                &payment.fees.to_string(),
                &i64::try_from(payment.timestamp)?,
                &Some(payment.method.to_string()),
                &withdraw_tx_id,
                &spark,
            ],
        )
        .await
        .map_err(map_db_error)?;

        match payment.details {
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                ..
            }) => {
                if invoice_details.is_some() || htlc_details.is_some() {
                    let invoice_json = to_json_opt(invoice_details.as_ref())?;
                    let htlc_json = to_json_opt(htlc_details.as_ref())?;
                    tx.execute(
                        "INSERT INTO brz_payment_details_spark (user_id, payment_id, invoice_details, htlc_details)
                             VALUES ($1, $2, $3, $4)
                             ON CONFLICT(user_id, payment_id) DO UPDATE SET
                                invoice_details = COALESCE(EXCLUDED.invoice_details, brz_payment_details_spark.invoice_details),
                                htlc_details = COALESCE(EXCLUDED.htlc_details, brz_payment_details_spark.htlc_details)",
                        &[&identity, &payment.id, &invoice_json, &htlc_json],
                    )
                    .await
                    .map_err(map_db_error)?;
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
                    "INSERT INTO brz_payment_details_token (user_id, payment_id, metadata, tx_hash, tx_type, invoice_details)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT(user_id, payment_id) DO UPDATE SET
                            metadata = EXCLUDED.metadata,
                            tx_hash = EXCLUDED.tx_hash,
                            tx_type = EXCLUDED.tx_type,
                            invoice_details = COALESCE(EXCLUDED.invoice_details, brz_payment_details_token.invoice_details)",
                    &[&identity, &payment.id, &metadata_json, &tx_hash, &tx_type.to_string(), &invoice_json],
                )
                .await
                .map_err(map_db_error)?;
            }
            Some(PaymentDetails::Lightning {
                invoice,
                destination_pubkey,
                description,
                htlc_details,
                ..
            }) => {
                let payment_hash = &htlc_details.payment_hash;
                let preimage = &htlc_details.preimage;
                let htlc_status = htlc_details.status.to_string();
                let htlc_expiry_time = i64::try_from(htlc_details.expiry_time)?;
                tx.execute(
                    "INSERT INTO brz_payment_details_lightning (user_id, payment_id, invoice, payment_hash, destination_pubkey, description, preimage, htlc_status, htlc_expiry_time)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                         ON CONFLICT(user_id, payment_id) DO UPDATE SET
                            invoice = EXCLUDED.invoice,
                            payment_hash = EXCLUDED.payment_hash,
                            destination_pubkey = EXCLUDED.destination_pubkey,
                            description = EXCLUDED.description,
                            preimage = COALESCE(EXCLUDED.preimage, brz_payment_details_lightning.preimage),
                            htlc_status = COALESCE(EXCLUDED.htlc_status, brz_payment_details_lightning.htlc_status),
                            htlc_expiry_time = COALESCE(EXCLUDED.htlc_expiry_time, brz_payment_details_lightning.htlc_expiry_time)",
                    &[&identity, &payment.id, &invoice, payment_hash, &destination_pubkey, &description, preimage, &htlc_status, &htlc_expiry_time],
                )
                .await
                .map_err(map_db_error)?;
            }
            Some(PaymentDetails::Deposit { tx_id, vout }) => {
                tx.execute(
                    "INSERT INTO brz_payment_details_deposit (user_id, payment_id, tx_id, vout)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT(user_id, payment_id) DO UPDATE SET
                            tx_id = EXCLUDED.tx_id,
                            vout = EXCLUDED.vout",
                    &[&identity, &payment.id, &tx_id, &i64::from(vout)],
                )
                .await
                .map_err(map_db_error)?;
            }
            // Withdraw detail columns are already set in the main INSERT
            Some(PaymentDetails::Withdraw { .. }) | None => {}
        }

        Ok(())
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    async fn list_payments(
        &self,
        request: StorageListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        // Build WHERE clauses based on filters. Tenant scoping is always $1; subsequent
        // dynamic filters use $2 onward.
        let mut where_clauses = vec!["p.user_id = $1".to_string()];
        let mut params: Vec<Box<dyn ToSql + Sync + Send>> = vec![Box::new(self.identity.clone())];
        let mut param_idx = 2;

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
                // Filter by HTLC status (Spark or Lightning)
                let htlc_filter = match payment_details_filter {
                    StoragePaymentDetailsFilter::Spark {
                        htlc_status: Some(s),
                        ..
                    } if !s.is_empty() => Some(("s", s)),
                    StoragePaymentDetailsFilter::Lightning {
                        htlc_status: Some(s),
                        ..
                    } if !s.is_empty() => Some(("l", s)),
                    _ => None,
                };
                if let Some((alias, htlc_statuses)) = htlc_filter {
                    let placeholders: Vec<String> = htlc_statuses
                        .iter()
                        .map(|_| {
                            let placeholder = format!("${param_idx}");
                            param_idx += 1;
                            placeholder
                        })
                        .collect();
                    if alias == "l" {
                        // Lightning: htlc_status is a direct column
                        payment_details_clauses
                            .push(format!("l.htlc_status IN ({})", placeholders.join(", ")));
                    } else {
                        // Spark: htlc_details is still JSONB
                        payment_details_clauses.push(format!(
                            "s.htlc_details::jsonb->>'status' IN ({})",
                            placeholders.join(", ")
                        ));
                    }
                    for htlc_status in htlc_statuses {
                        params.push(Box::new(htlc_status.to_string()));
                    }
                }
                // Payment type discriminator
                match payment_details_filter {
                    StoragePaymentDetailsFilter::Spark { .. } => {
                        payment_details_clauses.push("p.spark = true".to_string());
                    }
                    StoragePaymentDetailsFilter::Token { .. } => {
                        payment_details_clauses.push("p.spark IS NULL".to_string());
                    }
                    StoragePaymentDetailsFilter::Lightning { .. } => {}
                }

                // Filter by conversion info type + status
                let conversion_filter = match payment_details_filter {
                    StoragePaymentDetailsFilter::Spark {
                        conversion_filter: Some(cf),
                        ..
                    }
                    | StoragePaymentDetailsFilter::Token {
                        conversion_filter: Some(cf),
                        ..
                    }
                    | StoragePaymentDetailsFilter::Lightning {
                        conversion_filter: Some(cf),
                        ..
                    } => Some(cf),
                    _ => None,
                };
                if let Some(cf) = conversion_filter {
                    let status_clause = match cf {
                        crate::persist::ConversionFilter::AmmRefundNeeded => {
                            "pm.conversion_info::jsonb->>'type' = 'amm' AND \
                             pm.conversion_info::jsonb->>'status' = 'RefundNeeded'"
                        }
                        crate::persist::ConversionFilter::OrchestraPending => {
                            "pm.conversion_info::jsonb->>'type' = 'orchestra' AND \
                             pm.conversion_info::jsonb->>'status' NOT IN ('Completed', 'Failed', 'Refunded')"
                        }
                        crate::persist::ConversionFilter::BoltzPending => {
                            "pm.conversion_info::jsonb->>'type' = 'boltz' AND \
                             pm.conversion_info::jsonb->>'status' NOT IN ('Completed', 'Failed', 'Refunded')"
                        }
                    };
                    payment_details_clauses.push(format!(
                        "pm.conversion_info IS NOT NULL AND {status_clause}"
                    ));
                }
                // Filter by token transaction hash
                if let StoragePaymentDetailsFilter::Token {
                    tx_hash: Some(tx_hash),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push(format!("t.tx_hash = ${param_idx}"));
                    param_idx += 1;
                    params.push(Box::new(tx_hash.clone()));
                }
                // Filter by token transaction type
                if let StoragePaymentDetailsFilter::Token {
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

        // Build the WHERE clause (always non-empty: tenant scoping is the first clause)
        let where_sql = format!("WHERE {}", where_clauses.join(" AND "));

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

    async fn apply_payment_update(&self, payment: Payment) -> Result<bool, StorageError> {
        let mut client = self.pool.get().await.map_err(map_pool_error)?;
        let tx = client.transaction().await.map_err(map_db_error)?;
        let payment_lock_key = Self::payment_update_lock_key(&self.identity, &payment.id);
        tx.execute("SELECT pg_advisory_xact_lock($1)", &[&payment_lock_key])
            .await
            .map_err(map_db_error)?;
        let stored_status =
            Self::get_payment_status_in_tx(&tx, &self.identity, &payment.id).await?;

        // Guard against downgrading a terminal status.
        if let Some(stored) = stored_status
            && stored.is_final()
            && stored != payment.status
        {
            warn!(
                "Skipping payment update (would replace terminal status): id={} stored={stored:?} new={:?}",
                payment.id, payment.status
            );
            tx.commit().await.map_err(map_db_error)?;
            return Ok(false);
        }

        let same_status = stored_status == Some(payment.status);
        if same_status {
            tracing::debug!(
                "Skipping redundant payment event: id={} status={:?}",
                payment.id,
                payment.status
            );
        }
        Self::insert_payment_in_tx(&tx, &self.identity, payment).await?;
        tx.commit().await.map_err(map_db_error)?;
        Ok(!same_status)
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
        let conversion_status_str = metadata
            .conversion_status
            .as_ref()
            .map(std::string::ToString::to_string);

        client
            .execute(
                "INSERT INTO brz_payment_metadata (user_id, payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info, conversion_status)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT(user_id, payment_id) DO UPDATE SET
                    parent_payment_id = COALESCE(EXCLUDED.parent_payment_id, brz_payment_metadata.parent_payment_id),
                    lnurl_pay_info = COALESCE(EXCLUDED.lnurl_pay_info, brz_payment_metadata.lnurl_pay_info),
                    lnurl_withdraw_info = COALESCE(EXCLUDED.lnurl_withdraw_info, brz_payment_metadata.lnurl_withdraw_info),
                    lnurl_description = COALESCE(EXCLUDED.lnurl_description, brz_payment_metadata.lnurl_description),
                    conversion_info = COALESCE(EXCLUDED.conversion_info, brz_payment_metadata.conversion_info),
                    conversion_status = COALESCE(EXCLUDED.conversion_status, brz_payment_metadata.conversion_status)",
                &[
                    &self.identity,
                    &payment_id,
                    &metadata.parent_payment_id,
                    &lnurl_pay_info_json,
                    &lnurl_withdraw_info_json,
                    &metadata.lnurl_description,
                    &conversion_info_json,
                    &conversion_status_str,
                ],
            )
            .await?;

        Ok(())
    }

    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        client
            .execute(
                "INSERT INTO brz_settings (user_id, key, value) VALUES ($1, $2, $3)
                 ON CONFLICT(user_id, key) DO UPDATE SET value = EXCLUDED.value",
                &[&self.identity, &key, &value],
            )
            .await?;

        Ok(())
    }

    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        let row = client
            .query_opt(
                "SELECT value FROM brz_settings WHERE user_id = $1 AND key = $2",
                &[&self.identity, &key],
            )
            .await?;

        Ok(row.map(|r| r.get(0)))
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;

        client
            .execute(
                "DELETE FROM brz_settings WHERE user_id = $1 AND key = $2",
                &[&self.identity, &key],
            )
            .await?;

        Ok(())
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.user_id = $1 AND p.id = $2");
        let row = client
            .query_one(&query, &[&self.identity, &id])
            .await
            .map_err(map_db_error)?;
        map_payment(&row)
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.user_id = $1 AND l.invoice = $2");
        let row = client
            .query_opt(&query, &[&self.identity, &invoice])
            .await?;

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

        // Early exit if no related payments exist for this tenant
        let has_related: bool = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM brz_payment_metadata WHERE user_id = $1 AND parent_payment_id IS NOT NULL LIMIT 1)",
                &[&self.identity],
            )
            .await
            .is_ok_and(|row| row.get(0));

        if !has_related {
            return Ok(HashMap::new());
        }

        // Build the IN clause with placeholders. $1 is reserved for user_id; parent ids
        // start at $2.
        let placeholders: Vec<String> = parent_payment_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 2))
            .collect();
        let in_clause = placeholders.join(", ");

        let query = format!(
            "{SELECT_PAYMENT_SQL} WHERE p.user_id = $1 AND pm.parent_payment_id IN ({in_clause}) ORDER BY p.timestamp ASC"
        );

        let mut params: Vec<&(dyn ToSql + Sync)> = vec![&self.identity];
        params.extend(
            parent_payment_ids
                .iter()
                .map(|id| id as &(dyn ToSql + Sync)),
        );

        let rows = client.query(&query, &params).await?;

        let mut result: HashMap<String, Vec<Payment>> = HashMap::new();
        for row in rows {
            let payment = map_payment(&row)?;
            let parent_payment_id: String = row.get(32);
            result.entry(parent_payment_id).or_default().push(payment);
        }

        Ok(result)
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
        is_mature: bool,
    ) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        client
            .execute(
                "INSERT INTO brz_unclaimed_deposits (user_id, txid, vout, amount_sats, is_mature)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(user_id, txid, vout) DO UPDATE SET is_mature = EXCLUDED.is_mature, amount_sats = EXCLUDED.amount_sats",
                &[
                    &self.identity,
                    &txid,
                    &i32::try_from(vout)?,
                    &i64::try_from(amount_sats)?,
                    &is_mature,
                ],
            )
            .await?;
        Ok(())
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        client
            .execute(
                "DELETE FROM brz_unclaimed_deposits WHERE user_id = $1 AND txid = $2 AND vout = $3",
                &[&self.identity, &txid, &i32::try_from(vout)?],
            )
            .await?;
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let rows = client
            .query(
                "SELECT txid, vout, amount_sats, is_mature, claim_error, refund_tx, refund_tx_id FROM brz_unclaimed_deposits WHERE user_id = $1",
                &[&self.identity],
            )
            .await?;

        let mut deposits = Vec::new();
        for row in rows {
            let claim_error_json: Option<serde_json::Value> = row.get(4);
            let claim_error: Option<DepositClaimError> = from_json_opt(claim_error_json)?;

            deposits.push(DepositInfo {
                txid: row.get(0),
                vout: u32::try_from(row.get::<_, i32>(1))?,
                amount_sats: row
                    .get::<_, Option<i64>>(2)
                    .map(u64::try_from)
                    .transpose()?
                    .unwrap_or(0),
                is_mature: row.get(3),
                claim_error,
                refund_tx: row.get(5),
                refund_tx_id: row.get(6),
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
                        "UPDATE brz_unclaimed_deposits SET claim_error = $1, refund_tx = NULL, refund_tx_id = NULL WHERE user_id = $2 AND txid = $3 AND vout = $4",
                        &[&error_json, &self.identity, &txid, &i32::try_from(vout)?],
                    )
                    .await?;
            }
            UpdateDepositPayload::Refund {
                refund_txid,
                refund_tx,
            } => {
                client
                    .execute(
                        "UPDATE brz_unclaimed_deposits SET refund_tx = $1, refund_tx_id = $2, claim_error = NULL WHERE user_id = $3 AND txid = $4 AND vout = $5",
                        &[&refund_tx, &refund_txid, &self.identity, &txid, &i32::try_from(vout)?],
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
                    "INSERT INTO brz_lnurl_receive_metadata (user_id, payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT(user_id, payment_hash) DO UPDATE SET
                        nostr_zap_request = EXCLUDED.nostr_zap_request,
                        nostr_zap_receipt = EXCLUDED.nostr_zap_receipt,
                        sender_comment = EXCLUDED.sender_comment",
                    &[&self.identity, &m.payment_hash, &m.nostr_zap_request, &m.nostr_zap_receipt, &m.sender_comment],
                )
                .await?;
        }
        Ok(())
    }

    async fn list_contacts(
        &self,
        request: ListContactsRequest,
    ) -> Result<Vec<Contact>, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let limit = i64::from(request.limit.unwrap_or(u32::MAX));
        let offset = i64::from(request.offset.unwrap_or(0));

        let rows = client
            .query(
                "SELECT id, name, payment_identifier, created_at, updated_at
                 FROM brz_contacts WHERE user_id = $1 ORDER BY name ASC LIMIT $2 OFFSET $3",
                &[&self.identity, &limit, &offset],
            )
            .await?;

        let mut contacts = Vec::new();
        for row in rows {
            contacts.push(Contact {
                id: row.get(0),
                name: row.get(1),
                payment_identifier: row.get(2),
                created_at: u64::try_from(row.get::<_, i64>(3))?,
                updated_at: u64::try_from(row.get::<_, i64>(4))?,
            });
        }
        Ok(contacts)
    }

    async fn get_contact(&self, id: String) -> Result<Contact, StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let row = client
            .query_opt(
                "SELECT id, name, payment_identifier, created_at, updated_at
                 FROM brz_contacts WHERE user_id = $1 AND id = $2",
                &[&self.identity, &id],
            )
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(Contact {
            id: row.get(0),
            name: row.get(1),
            payment_identifier: row.get(2),
            created_at: u64::try_from(row.get::<_, i64>(3))?,
            updated_at: u64::try_from(row.get::<_, i64>(4))?,
        })
    }

    async fn insert_contact(&self, contact: Contact) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        let result = client
            .execute(
                "INSERT INTO brz_contacts (user_id, id, name, payment_identifier, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (user_id, id) DO UPDATE SET
                   name = EXCLUDED.name,
                   payment_identifier = EXCLUDED.payment_identifier,
                   updated_at = EXCLUDED.updated_at",
                &[
                    &self.identity,
                    &contact.id,
                    &contact.name,
                    &contact.payment_identifier,
                    &i64::try_from(contact.created_at)?,
                    &i64::try_from(contact.updated_at)?,
                ],
            )
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(map_db_error(e)),
        }
    }

    async fn delete_contact(&self, id: String) -> Result<(), StorageError> {
        let client = self.pool.get().await.map_err(map_pool_error)?;
        client
            .execute(
                "DELETE FROM brz_contacts WHERE user_id = $1 AND id = $2",
                &[&self.identity, &id],
            )
            .await?;
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
        // Scoped per-tenant so two tenants don't share a queue.
        let local_revision: i64 = tx
            .query_one(
                "SELECT COALESCE(MAX(revision), 0) + 1 FROM brz_sync_outgoing WHERE user_id = $1",
                &[&self.identity],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?
            .get(0);

        let updated_fields_json = serde_json::to_value(&record.updated_fields)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.execute(
            "INSERT INTO brz_sync_outgoing (user_id, record_type, data_id, schema_version, commit_time, updated_fields_json, revision)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &self.identity,
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

        // Delete from sync_outgoing using local_revision (the change's revision number)
        let tx = client
            .transaction()
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let rows_deleted = tx
            .execute(
                "DELETE FROM brz_sync_outgoing WHERE user_id = $1 AND record_type = $2 AND data_id = $3 AND revision = $4",
                &[
                    &self.identity,
                    &record.id.r#type,
                    &record.id.data_id,
                    &i64::try_from(local_revision)?,
                ],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        if rows_deleted == 0 {
            warn!(
                "complete_outgoing_sync: DELETE from brz_sync_outgoing matched 0 rows \
                 (type={}, data_id={}, revision={})",
                record.id.r#type, record.id.data_id, local_revision
            );
        }

        let data_json = serde_json::to_value(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.execute(
            "INSERT INTO brz_sync_state (user_id, record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(user_id, record_type, data_id) DO UPDATE SET
                    schema_version = EXCLUDED.schema_version,
                    commit_time = EXCLUDED.commit_time,
                    data = EXCLUDED.data,
                    revision = EXCLUDED.revision",
            &[
                &self.identity,
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

        // Upsert this tenant's revision row. The migration creates a row at backfill, but
        // a fresh tenant joining a shared DB after migration won't have one yet.
        tx.execute(
            "INSERT INTO brz_sync_revision (user_id, revision) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET revision = GREATEST(brz_sync_revision.revision, EXCLUDED.revision)",
            &[&self.identity, &i64::try_from(record.revision)?],
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
                 FROM brz_sync_outgoing o
                 LEFT JOIN brz_sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id AND o.user_id = e.user_id
                 WHERE o.user_id = $1
                 ORDER BY o.revision ASC
                 LIMIT $2",
                &[&self.identity, &i64::from(limit)],
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

        // A tenant that hasn't synced anything yet may not have a row. Treat missing as 0.
        let revision: i64 = client
            .query_opt(
                "SELECT revision FROM brz_sync_revision WHERE user_id = $1",
                &[&self.identity],
            )
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?
            .map_or(0, |row| row.get(0));

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
                    "INSERT INTO brz_sync_incoming (user_id, record_type, data_id, schema_version, commit_time, data, revision)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT(user_id, record_type, data_id, revision) DO UPDATE SET
                        schema_version = EXCLUDED.schema_version,
                        commit_time = EXCLUDED.commit_time,
                        data = EXCLUDED.data",
                    &[
                        &self.identity,
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
                "DELETE FROM brz_sync_incoming WHERE user_id = $1 AND record_type = $2 AND data_id = $3 AND revision = $4",
                &[
                    &self.identity,
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
                 FROM brz_sync_incoming i
                 LEFT JOIN brz_sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id AND i.user_id = e.user_id
                 WHERE i.user_id = $1
                 ORDER BY i.revision ASC
                 LIMIT $2",
                &[&self.identity, &i64::from(limit)],
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
                 FROM brz_sync_outgoing o
                 LEFT JOIN brz_sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id AND o.user_id = e.user_id
                 WHERE o.user_id = $1
                 ORDER BY o.revision DESC
                 LIMIT 1",
                &[&self.identity],
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
            "INSERT INTO brz_sync_state (user_id, record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(user_id, record_type, data_id) DO UPDATE SET
                    schema_version = EXCLUDED.schema_version,
                    commit_time = EXCLUDED.commit_time,
                    data = EXCLUDED.data,
                    revision = EXCLUDED.revision",
            &[
                &self.identity,
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

        // Upsert this tenant's revision row.
        tx.execute(
            "INSERT INTO brz_sync_revision (user_id, revision) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET revision = GREATEST(brz_sync_revision.revision, EXCLUDED.revision)",
            &[&self.identity, &i64::try_from(record.revision)?],
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
/// Column indices 0-31 are used by `map_payment`, index 32 (`parent_payment_id`) is only used by `get_payments_by_parent_ids`.
const SELECT_PAYMENT_SQL: &str = "
    SELECT p.id,
           p.payment_type,
           p.status,
           p.amount,
           p.fees,
           p.timestamp,
           p.method,
           p.withdraw_tx_id,
           pd.tx_id AS deposit_tx_id,
           pd.vout AS deposit_vout,
           p.spark,
           l.invoice AS lightning_invoice,
           l.payment_hash AS lightning_payment_hash,
           l.destination_pubkey AS lightning_destination_pubkey,
           COALESCE(l.description, pm.lnurl_description) AS lightning_description,
           l.preimage AS lightning_preimage,
           l.htlc_status AS lightning_htlc_status,
           l.htlc_expiry_time AS lightning_htlc_expiry_time,
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
           lrm.payment_hash AS lnurl_payment_hash,
           pm.conversion_status,
           pm.parent_payment_id
      FROM brz_payments p
      LEFT JOIN brz_payment_details_lightning l ON p.id = l.payment_id AND p.user_id = l.user_id
      LEFT JOIN brz_payment_details_token t ON p.id = t.payment_id AND p.user_id = t.user_id
      LEFT JOIN brz_payment_details_spark s ON p.id = s.payment_id AND p.user_id = s.user_id
      LEFT JOIN brz_payment_details_deposit pd ON p.id = pd.payment_id AND p.user_id = pd.user_id
      LEFT JOIN brz_payment_metadata pm ON p.id = pm.payment_id AND p.user_id = pm.user_id
      LEFT JOIN brz_lnurl_receive_metadata lrm ON l.payment_hash = lrm.payment_hash AND l.user_id = lrm.user_id";

#[allow(clippy::too_many_lines)]
fn map_payment(row: &Row) -> Result<Payment, StorageError> {
    let withdraw_tx_id: Option<String> = row.get(7);
    let deposit_tx_id: Option<String> = row.get(8);
    let spark: Option<bool> = row.get(10);
    let lightning_invoice: Option<String> = row.get(11);
    let token_metadata: Option<serde_json::Value> = row.get(21);

    let details = match (
        lightning_invoice,
        withdraw_tx_id,
        deposit_tx_id,
        spark,
        token_metadata,
    ) {
        (Some(invoice), _, _, _, _) => {
            let payment_hash: String = row.get(12);
            let destination_pubkey: String = row.get(13);
            let description: Option<String> = row.get(14);
            let preimage: Option<String> = row.get(15);
            let htlc_status_str: Option<String> = row.get(16);
            let htlc_status: SparkHtlcStatus = htlc_status_str
                .ok_or_else(|| {
                    StorageError::Implementation(
                        "htlc_status is required for Lightning payments".to_string(),
                    )
                })
                .and_then(|s| {
                    s.parse()
                        .map_err(|e: String| StorageError::Serialization(e))
                })?;
            let htlc_expiry_time: i64 = row.get(17);
            let htlc_details = SparkHtlcDetails {
                payment_hash,
                preimage,
                expiry_time: u64::try_from(htlc_expiry_time)?,
                status: htlc_status,
            };
            let lnurl_pay_info_json: Option<serde_json::Value> = row.get(18);
            let lnurl_withdraw_info_json: Option<serde_json::Value> = row.get(19);
            let lnurl_nostr_zap_request: Option<String> = row.get(27);
            let lnurl_nostr_zap_receipt: Option<String> = row.get(28);
            let lnurl_sender_comment: Option<String> = row.get(29);
            let lnurl_payment_hash: Option<String> = row.get(30);

            let lnurl_pay_info: Option<LnurlPayInfo> = from_json_opt(lnurl_pay_info_json)?;
            let lnurl_withdraw_info: Option<LnurlWithdrawInfo> =
                from_json_opt(lnurl_withdraw_info_json)?;

            let lnurl_receive_metadata = if lnurl_payment_hash.is_some() {
                Some(LnurlReceiveMetadata {
                    nostr_zap_request: lnurl_nostr_zap_request,
                    nostr_zap_receipt: lnurl_nostr_zap_receipt,
                    sender_comment: lnurl_sender_comment,
                })
            } else {
                None
            };
            let conversion_info_json: Option<serde_json::Value> = row.get(20);
            let conversion_info: Option<ConversionInfo> = from_json_opt(conversion_info_json)?;
            Some(PaymentDetails::Lightning {
                invoice,
                destination_pubkey,
                description,
                htlc_details,
                lnurl_pay_info,
                lnurl_withdraw_info,
                lnurl_receive_metadata,
                conversion_info,
            })
        }
        (_, Some(tx_id), _, _, _) => Some(PaymentDetails::Withdraw { tx_id }),
        (_, _, Some(tx_id), _, _) => {
            let vout: i64 = row.get::<_, Option<i64>>(9).ok_or_else(|| {
                StorageError::Serialization("deposit row missing deposit_vout".to_string())
            })?;
            Some(PaymentDetails::Deposit {
                tx_id,
                vout: u32::try_from(vout).map_err(|e| {
                    StorageError::Serialization(format!("invalid deposit_vout: {e}"))
                })?,
            })
        }
        (_, _, _, Some(_), _) => {
            let invoice_details_json: Option<serde_json::Value> = row.get(25);
            let invoice_details = from_json_opt(invoice_details_json)?;
            let htlc_details_json: Option<serde_json::Value> = row.get(26);
            let htlc_details = from_json_opt(htlc_details_json)?;
            let conversion_info_json: Option<serde_json::Value> = row.get(20);
            let conversion_info: Option<ConversionInfo> = from_json_opt(conversion_info_json)?;
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                conversion_info,
            })
        }
        (_, _, _, _, Some(metadata)) => {
            let tx_type_str: String = row.get(23);
            let tx_type = tx_type_str
                .parse()
                .map_err(|e: String| StorageError::Serialization(e))?;
            let invoice_details_json: Option<serde_json::Value> = row.get(24);
            let invoice_details = from_json_opt(invoice_details_json)?;
            let conversion_info_json: Option<serde_json::Value> = row.get(20);
            let conversion_info: Option<ConversionInfo> = from_json_opt(conversion_info_json)?;
            Some(PaymentDetails::Token {
                metadata: serde_json::from_value(metadata)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                tx_hash: row.get(22),
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
        conversion_details: {
            let conversion_status_str: Option<String> = row.get(31);
            conversion_status_str
                .map(|s| {
                    s.parse::<ConversionStatus>()
                        .map(|status| ConversionDetails {
                            status,
                            conversions: vec![],
                        })
                        .map_err(StorageError::Serialization)
                })
                .transpose()?
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spark_postgres::pool::parse_pem_to_root_store;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;

    /// Helper struct that holds the container and storage together.
    /// The container must be kept alive for the duration of the test.
    struct PostgresTestFixture {
        storage: PostgresStorage,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    /// A fixed 33-byte test identity used by single-tenant test fixtures.
    /// Two-tenant isolation tests use a different identity for the second tenant.
    pub(super) const TEST_IDENTITY_A: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

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

            let storage = PostgresStorage::new(
                PostgresStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY_A,
            )
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
    async fn test_lightning_htlc_details_and_status_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_lightning_htlc_details_and_status_filtering(Box::new(
            fixture.storage,
        ))
        .await;
    }

    #[tokio::test]
    async fn test_conversion_filtering() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_conversion_filtering(Box::new(fixture.storage)).await;
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
    async fn test_payment_terminal_status_is_not_replaced() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_payment_terminal_status_is_not_replaced(Box::new(
            fixture.storage,
        ))
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

    #[tokio::test]
    async fn test_contacts_crud() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_contacts_crud(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_conversion_status_persistence() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_conversion_status_persistence(Box::new(fixture.storage)).await;
    }

    /// Simulates the post-migration state for a legacy deposit: a row exists in
    /// `brz_payments` with `method = 'deposit'` but no matching
    /// `brz_payment_details_deposit` row (the SSP `user_request` hasn't been
    /// re-fetched yet). `list_payments` must return the payment with
    /// `details: None` and `method: Deposit` preserved.
    #[tokio::test]
    async fn test_legacy_deposit_without_details_row_returns_none() {
        use crate::PaymentMethod;
        use crate::persist::StorageListPaymentsRequest;

        let fixture = PostgresTestFixture::new().await;

        // Insert a deposit brz_payments row directly via the pool, bypassing
        // insert_payment_in_tx so no brz_payment_details_deposit row is written.
        let client = fixture.storage.pool.get().await.expect("get_client");
        client
            .execute(
                "INSERT INTO brz_payments
                 (user_id, id, payment_type, status, amount, fees, timestamp, method)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &TEST_IDENTITY_A.to_vec(),
                    &"legacy-deposit-1",
                    &"receive",
                    &"completed",
                    &"1000",
                    &"0",
                    &1_000_i64,
                    &"deposit",
                ],
            )
            .await
            .expect("seed legacy deposit");

        let payments = fixture
            .storage
            .list_payments(StorageListPaymentsRequest::default())
            .await
            .expect("list_payments");

        let p = payments
            .iter()
            .find(|p| p.id == "legacy-deposit-1")
            .expect("legacy deposit must appear in list_payments");
        assert!(
            p.details.is_none(),
            "legacy deposit must surface with details: None, got {:?}",
            p.details
        );
        assert_eq!(p.method, PaymentMethod::Deposit);
    }

    /// A second 33-byte test identity (must differ from `TEST_IDENTITY_A`).
    const TEST_IDENTITY_B: [u8; 33] = [
        0x03, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae,
        0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd,
        0xbe, 0xbf, 0xc0,
    ];

    /// Two `PostgresStorage` instances with distinct identities sharing one
    /// connection pool / DB. The container must be kept alive for the test.
    struct TwoTenantFixture {
        a: PostgresStorage,
        b: PostgresStorage,
        #[allow(dead_code)]
        container: ContainerAsync<Postgres>,
    }

    impl TwoTenantFixture {
        async fn new() -> Self {
            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start PostgreSQL container");

            let host_port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get host port");

            let connection_string = format!(
                "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
            );

            let config = PostgresStorageConfig::with_defaults(connection_string);
            let pool = create_pool(&config).expect("Failed to create pool");

            let a = PostgresStorage::new_with_pool(pool.clone(), &TEST_IDENTITY_A, true)
                .await
                .expect("Failed to create tenant A");
            let b = PostgresStorage::new_with_pool(pool, &TEST_IDENTITY_B, true)
                .await
                .expect("Failed to create tenant B");

            Self { a, b, container }
        }
    }

    /// End-to-end isolation: every Storage method must keep tenants A and B
    /// from observing each other's data. The test exercises each per-user
    /// table — `brz_payments`, `brz_payment_metadata`, `brz_lnurl_receive_metadata`,
    /// `brz_contacts`, `brz_unclaimed_deposits`, `brz_settings`, and the sync mirror
    /// tables — and asserts that writes by A are invisible to B (and vice
    /// versa). It is the regression net for "forgot the WHERE clause" bugs
    /// in any future query.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_two_tenant_isolation() {
        use crate::models::{Contact, ListContactsRequest};
        use crate::persist::{Payment, StorageListPaymentsRequest};
        use crate::sync_storage::{Record, RecordId, UnversionedRecordChange};
        use crate::{
            PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SetLnurlMetadataItem,
            SparkHtlcDetails, SparkHtlcStatus, Storage,
        };
        use std::collections::HashMap;

        let fx = TwoTenantFixture::new().await;

        // --- payments (incl. lightning details) ---
        let pmt_a = Payment {
            id: "pmt_shared_id".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 1_000,
            fees: 10,
            timestamp: 100,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                invoice: "lnbc_a".to_string(),
                destination_pubkey: "pkA".to_string(),
                description: None,
                htlc_details: SparkHtlcDetails {
                    payment_hash: "shared_payment_hash".to_string(),
                    preimage: Some("preimage_a".to_string()),
                    expiry_time: 0,
                    status: SparkHtlcStatus::PreimageShared,
                },
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: None,
            }),
            conversion_details: None,
        };
        let mut pmt_b = pmt_a.clone();
        if let Some(PaymentDetails::Lightning {
            invoice,
            destination_pubkey,
            ..
        }) = &mut pmt_b.details
        {
            *invoice = "lnbc_b".to_string();
            *destination_pubkey = "pkB".to_string();
        }

        fx.a.apply_payment_update(pmt_a.clone()).await.unwrap();
        fx.b.apply_payment_update(pmt_b.clone()).await.unwrap();

        // Each tenant's list contains only its own row.
        let list_a =
            fx.a.list_payments(StorageListPaymentsRequest::default())
                .await
                .unwrap();
        let list_b =
            fx.b.list_payments(StorageListPaymentsRequest::default())
                .await
                .unwrap();
        assert_eq!(list_a.len(), 1, "tenant A should see exactly 1 payment");
        assert_eq!(list_b.len(), 1, "tenant B should see exactly 1 payment");
        if let Some(PaymentDetails::Lightning { invoice, .. }) = &list_a[0].details {
            assert_eq!(invoice, "lnbc_a");
        } else {
            panic!("expected lightning payment for A");
        }
        if let Some(PaymentDetails::Lightning { invoice, .. }) = &list_b[0].details {
            assert_eq!(invoice, "lnbc_b");
        } else {
            panic!("expected lightning payment for B");
        }

        // get_payment_by_id is per-tenant: same id, different details, no leakage.
        let by_id_a =
            fx.a.get_payment_by_id("pmt_shared_id".to_string())
                .await
                .unwrap();
        let by_id_b =
            fx.b.get_payment_by_id("pmt_shared_id".to_string())
                .await
                .unwrap();
        match (&by_id_a.details, &by_id_b.details) {
            (
                Some(PaymentDetails::Lightning { invoice: ia, .. }),
                Some(PaymentDetails::Lightning { invoice: ib, .. }),
            ) => assert!(ia != ib, "tenants must not see each other's invoice"),
            _ => panic!("expected lightning details for both"),
        }

        // get_payment_by_invoice is also per-tenant.
        assert!(
            fx.a.get_payment_by_invoice("lnbc_b".to_string())
                .await
                .unwrap()
                .is_none(),
            "tenant A must not find tenant B's invoice"
        );
        assert!(
            fx.b.get_payment_by_invoice("lnbc_a".to_string())
                .await
                .unwrap()
                .is_none(),
            "tenant B must not find tenant A's invoice"
        );

        // --- contacts ---
        let now = 0u64;
        fx.a.insert_contact(Contact {
            id: "shared_contact_id".to_string(),
            name: "Alice".to_string(),
            payment_identifier: "alice@a".to_string(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
        let b_contacts =
            fx.b.list_contacts(ListContactsRequest::default())
                .await
                .unwrap();
        assert!(
            b_contacts.is_empty(),
            "tenant B must not see tenant A's contact"
        );
        // get_contact for the shared id should return NotFound for B.
        assert!(
            fx.b.get_contact("shared_contact_id".to_string())
                .await
                .is_err(),
            "tenant B must not retrieve tenant A's contact by id"
        );

        // --- unclaimed deposits ---
        fx.a.add_deposit("shared_txid".to_string(), 0, 5_000, true)
            .await
            .unwrap();
        let b_deposits = fx.b.list_deposits().await.unwrap();
        assert!(
            b_deposits.is_empty(),
            "tenant B must not see tenant A's deposit"
        );

        // --- settings (cached items) ---
        fx.a.set_cached_item("k".to_string(), "value_a".to_string())
            .await
            .unwrap();
        fx.b.set_cached_item("k".to_string(), "value_b".to_string())
            .await
            .unwrap();
        assert_eq!(
            fx.a.get_cached_item("k".to_string()).await.unwrap(),
            Some("value_a".to_string())
        );
        assert_eq!(
            fx.b.get_cached_item("k".to_string()).await.unwrap(),
            Some("value_b".to_string())
        );
        // Deleting in B must not affect A.
        fx.b.delete_cached_item("k".to_string()).await.unwrap();
        assert_eq!(
            fx.a.get_cached_item("k".to_string()).await.unwrap(),
            Some("value_a".to_string())
        );
        assert_eq!(fx.b.get_cached_item("k".to_string()).await.unwrap(), None);

        // --- lnurl receive metadata ---
        fx.a.set_lnurl_metadata(vec![SetLnurlMetadataItem {
            payment_hash: "shared_payment_hash".to_string(),
            nostr_zap_request: Some("zap_a".to_string()),
            nostr_zap_receipt: None,
            sender_comment: None,
        }])
        .await
        .unwrap();
        fx.b.set_lnurl_metadata(vec![SetLnurlMetadataItem {
            payment_hash: "shared_payment_hash".to_string(),
            nostr_zap_request: Some("zap_b".to_string()),
            nostr_zap_receipt: None,
            sender_comment: None,
        }])
        .await
        .unwrap();
        // Each tenant's get_payment_by_id surfaces its own lnurl metadata via
        // the SELECT_PAYMENT_SQL JOIN — confirms the lrm join is user-scoped.
        let by_id_a =
            fx.a.get_payment_by_id("pmt_shared_id".to_string())
                .await
                .unwrap();
        let by_id_b =
            fx.b.get_payment_by_id("pmt_shared_id".to_string())
                .await
                .unwrap();
        if let (
            Some(PaymentDetails::Lightning {
                lnurl_receive_metadata: Some(ma),
                ..
            }),
            Some(PaymentDetails::Lightning {
                lnurl_receive_metadata: Some(mb),
                ..
            }),
        ) = (&by_id_a.details, &by_id_b.details)
        {
            assert_eq!(ma.nostr_zap_request.as_deref(), Some("zap_a"));
            assert_eq!(mb.nostr_zap_request.as_deref(), Some("zap_b"));
        } else {
            panic!("expected lnurl metadata to be visible to each tenant");
        }

        // --- sync state (brz_sync_outgoing, brz_sync_state, brz_sync_revision) ---
        let rec_id = RecordId::new("contact".to_string(), "rec_shared".to_string());
        let updated_a: HashMap<String, String> = HashMap::new();
        fx.a.add_outgoing_change(UnversionedRecordChange {
            id: rec_id.clone(),
            schema_version: "1".to_string(),
            updated_fields: updated_a,
        })
        .await
        .unwrap();
        // B's pending queue must be empty.
        let b_pending = fx.b.get_pending_outgoing_changes(100).await.unwrap();
        assert!(
            b_pending.is_empty(),
            "tenant B must not see tenant A's pending outgoing"
        );
        // B's revision must be 0 even after A's queue is populated.
        assert_eq!(fx.b.get_last_revision().await.unwrap(), 0);

        // A completes the change with revision 7; B's revision remains untouched.
        let rec = Record {
            id: rec_id.clone(),
            schema_version: "1".to_string(),
            data: HashMap::new(),
            revision: 7,
        };
        let a_pending = fx.a.get_pending_outgoing_changes(100).await.unwrap();
        let a_local_rev = a_pending[0].change.local_revision;
        fx.a.complete_outgoing_sync(rec.clone(), a_local_rev)
            .await
            .unwrap();
        assert_eq!(fx.a.get_last_revision().await.unwrap(), 7);
        assert_eq!(
            fx.b.get_last_revision().await.unwrap(),
            0,
            "tenant B's revision must remain isolated from tenant A's bumps"
        );

        // Incoming records: insert via A; B must not see them, and B's deletes
        // of an identical key must not affect A's.
        let rec_b = Record {
            id: rec_id.clone(),
            schema_version: "1".to_string(),
            data: HashMap::new(),
            revision: 11,
        };
        fx.a.insert_incoming_records(vec![rec_b.clone()])
            .await
            .unwrap();
        let b_incoming = fx.b.get_incoming_records(100).await.unwrap();
        assert!(
            b_incoming.is_empty(),
            "tenant B must not see tenant A's incoming records"
        );
        fx.b.delete_incoming_record(rec_b.clone()).await.unwrap(); // no-op for B
        let a_incoming = fx.a.get_incoming_records(100).await.unwrap();
        assert_eq!(
            a_incoming.len(),
            1,
            "tenant A's incoming must survive B's delete on the same key"
        );

        // --- final cross-check: tenant B's full payment list still has only its row ---
        let list_b_final =
            fx.b.list_payments(StorageListPaymentsRequest::default())
                .await
                .unwrap();
        assert_eq!(list_b_final.len(), 1);
        assert_eq!(list_b_final[0].id, "pmt_shared_id");
    }

    #[tokio::test]
    async fn test_insert_boltz_conversion_info() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_insert_boltz_conversion_info(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_update_boltz_status_to_completed() {
        let fixture = PostgresTestFixture::new().await;
        crate::persist::tests::test_update_boltz_status_to_completed(Box::new(fixture.storage))
            .await;
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

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_migration_htlc_details() {
        use crate::{
            PaymentDetails, SparkHtlcStatus, Storage,
            persist::{StorageListPaymentsRequest, StoragePaymentDetailsFilter},
        };

        // Start a PostgreSQL container
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );

        // Step 1: Connect directly and apply migrations 1-6 (before the htlc_status backfill)
        {
            let (client, conn) = tokio_postgres::connect(&connection_string, tokio_postgres::NoTls)
                .await
                .expect("Failed to connect");
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    eprintln!("connection error: {e}");
                }
            });

            // Create the brz_schema_migrations table
            client
                .execute(
                    "CREATE TABLE IF NOT EXISTS brz_schema_migrations (
                        version INTEGER PRIMARY KEY,
                        applied_at TIMESTAMPTZ DEFAULT NOW()
                    )",
                    &[],
                )
                .await
                .unwrap();

            // Apply migrations 1-6 (index 0-5)
            let migrations = PostgresStorage::migrations(&TEST_IDENTITY_A);
            for (i, migration) in migrations.iter().take(6).enumerate() {
                let version = i32::try_from(i + 1).unwrap();
                for statement in migration {
                    client.execute(statement.as_str(), &[]).await.unwrap();
                }
                client
                    .execute(
                        "INSERT INTO brz_schema_migrations (version) VALUES ($1)",
                        &[&version],
                    )
                    .await
                    .unwrap();
            }

            // Step 2: Insert Lightning payments with different statuses
            // Completed payment
            client
                .execute(
                    "INSERT INTO brz_payments (id, payment_type, status, amount, fees, timestamp, method)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &"ln-completed",
                        &"send",
                        &"completed",
                        &"1000",
                        &"10",
                        &1_700_000_001_i64,
                        &"\"lightning\"",
                    ],
                )
                .await
                .unwrap();
            client
                .execute(
                    "INSERT INTO brz_payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, preimage)
                     VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &"ln-completed",
                        &"lnbc_completed",
                        &"hash_completed_0123456789abcdef0123456789abcdef0123456789abcdef01234567",
                        &"03pubkey1",
                        &"preimage_completed",
                    ],
                )
                .await
                .unwrap();

            // Pending payment
            client
                .execute(
                    "INSERT INTO brz_payments (id, payment_type, status, amount, fees, timestamp, method)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &"ln-pending",
                        &"receive",
                        &"pending",
                        &"2000",
                        &"0",
                        &1_700_000_002_i64,
                        &"\"lightning\"",
                    ],
                )
                .await
                .unwrap();
            client
                .execute(
                    "INSERT INTO brz_payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey)
                     VALUES ($1, $2, $3, $4)",
                    &[
                        &"ln-pending",
                        &"lnbc_pending",
                        &"hash_pending_0123456789abcdef0123456789abcdef0123456789abcdef012345678",
                        &"03pubkey2",
                    ],
                )
                .await
                .unwrap();

            // Failed payment
            client
                .execute(
                    "INSERT INTO brz_payments (id, payment_type, status, amount, fees, timestamp, method)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &"ln-failed",
                        &"send",
                        &"failed",
                        &"3000",
                        &"5",
                        &1_700_000_003_i64,
                        &"\"lightning\"",
                    ],
                )
                .await
                .unwrap();
            client
                .execute(
                    "INSERT INTO brz_payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey)
                     VALUES ($1, $2, $3, $4)",
                    &[
                        &"ln-failed",
                        &"lnbc_failed",
                        &"hash_failed_0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                        &"03pubkey3",
                    ],
                )
                .await
                .unwrap();
        }

        // Step 3: Open with PostgresStorage (triggers migration 7 - the backfill)
        let storage = PostgresStorage::new(
            PostgresStorageConfig::with_defaults(connection_string),
            &TEST_IDENTITY_A,
        )
        .await
        .expect("Failed to create PostgresStorage");

        // Step 4: Verify Completed → PreimageShared
        let completed = storage
            .get_payment_by_id("ln-completed".to_string())
            .await
            .unwrap();
        match &completed.details {
            Some(PaymentDetails::Lightning { htlc_details, .. }) => {
                assert_eq!(htlc_details.status, SparkHtlcStatus::PreimageShared);
                assert_eq!(htlc_details.expiry_time, 0);
                assert_eq!(
                    htlc_details.payment_hash,
                    "hash_completed_0123456789abcdef0123456789abcdef0123456789abcdef01234567"
                );
                assert_eq!(htlc_details.preimage.as_deref(), Some("preimage_completed"));
            }
            _ => panic!("Expected Lightning payment details for ln-completed"),
        }

        // Step 5: Verify Pending → WaitingForPreimage
        let pending = storage
            .get_payment_by_id("ln-pending".to_string())
            .await
            .unwrap();
        match &pending.details {
            Some(PaymentDetails::Lightning { htlc_details, .. }) => {
                assert_eq!(htlc_details.status, SparkHtlcStatus::WaitingForPreimage);
                assert_eq!(htlc_details.expiry_time, 0);
                assert_eq!(
                    htlc_details.payment_hash,
                    "hash_pending_0123456789abcdef0123456789abcdef0123456789abcdef012345678"
                );
                assert!(htlc_details.preimage.is_none());
            }
            _ => panic!("Expected Lightning payment details for ln-pending"),
        }

        // Step 6: Verify Failed → Returned
        let failed = storage
            .get_payment_by_id("ln-failed".to_string())
            .await
            .unwrap();
        match &failed.details {
            Some(PaymentDetails::Lightning { htlc_details, .. }) => {
                assert_eq!(htlc_details.status, SparkHtlcStatus::Returned);
                assert_eq!(htlc_details.expiry_time, 0);
            }
            _ => panic!("Expected Lightning payment details for ln-failed"),
        }

        // Step 7: Verify filtering by htlc_status works on migrated data
        let waiting_payments = storage
            .list_payments(StorageListPaymentsRequest {
                payment_details_filter: Some(vec![StoragePaymentDetailsFilter::Lightning {
                    htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
                    conversion_filter: None,
                }]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(waiting_payments.len(), 1);
        assert_eq!(waiting_payments[0].id, "ln-pending");

        let preimage_shared = storage
            .list_payments(StorageListPaymentsRequest {
                payment_details_filter: Some(vec![StoragePaymentDetailsFilter::Lightning {
                    htlc_status: Some(vec![SparkHtlcStatus::PreimageShared]),
                    conversion_filter: None,
                }]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(preimage_shared.len(), 1);
        assert_eq!(preimage_shared[0].id, "ln-completed");

        let returned = storage
            .list_payments(StorageListPaymentsRequest {
                payment_details_filter: Some(vec![StoragePaymentDetailsFilter::Lightning {
                    htlc_status: Some(vec![SparkHtlcStatus::Returned]),
                    conversion_filter: None,
                }]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].id, "ln-failed");
    }

    /// Migration backfill: an untyped (pre-migration) AMM `conversion_info`
    /// row is upgraded to a tagged enum and reads back via the strict
    /// `from_json_string_opt::<ConversionInfo>` path that `list_payments` /
    /// `get_payment_by_id` use.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_migration_conversion_info_type_discriminator() {
        use crate::{ConversionInfo, ConversionStatus, PaymentDetails, Storage};

        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );

        // Bring the DB up to the state right before the discriminator backfill.
        let migrations = PostgresStorage::migrations(&TEST_IDENTITY_A);
        let backfill_index = migrations.len() - 1;
        {
            let (client, conn) = tokio_postgres::connect(&connection_string, tokio_postgres::NoTls)
                .await
                .expect("Failed to connect");
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    eprintln!("connection error: {e}");
                }
            });
            client
                .execute(
                    "CREATE TABLE IF NOT EXISTS brz_schema_migrations (
                        version INTEGER PRIMARY KEY,
                        applied_at TIMESTAMPTZ DEFAULT NOW()
                    )",
                    &[],
                )
                .await
                .unwrap();
            for (i, migration) in migrations.iter().take(backfill_index).enumerate() {
                let version = i32::try_from(i + 1).unwrap();
                for statement in migration {
                    client.execute(statement.as_str(), &[]).await.unwrap();
                }
                client
                    .execute(
                        "INSERT INTO brz_schema_migrations (version) VALUES ($1)",
                        &[&version],
                    )
                    .await
                    .unwrap();
            }

            // Insert a Spark payment + an untyped (pre-migration) AMM row.
            // user_id is required (multi-tenant scoping migration is in the
            // applied set above).
            let user_id_bytes: &[u8] = &TEST_IDENTITY_A;
            client
                .execute(
                    "INSERT INTO brz_payments (user_id, id, payment_type, status, amount, fees, timestamp, method, spark)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                    &[
                        &user_id_bytes,
                        &"conv-migration-test",
                        &"send",
                        &"completed",
                        &"5000",
                        &"10",
                        &1_700_000_001_i64,
                        &"\"spark\"",
                        &true,
                    ],
                )
                .await
                .unwrap();
            client
                .execute(
                    "INSERT INTO brz_payment_details_spark (user_id, payment_id) VALUES ($1, $2)",
                    &[&user_id_bytes, &"conv-migration-test"],
                )
                .await
                .unwrap();
            let untyped = serde_json::json!({
                "pool_id": "pool-pre",
                "conversion_id": "conv-pre",
                "status": "Completed",
                "fee": "42",
                "purpose": null,
            });
            client
                .execute(
                    "INSERT INTO brz_payment_metadata (user_id, payment_id, conversion_info)
                     VALUES ($1, $2, $3)",
                    &[&user_id_bytes, &"conv-migration-test", &untyped],
                )
                .await
                .unwrap();
        }

        // Now open via PostgresStorage to trigger the remaining backfill migration.
        let config = PostgresStorageConfig::with_defaults(connection_string);
        let storage = PostgresStorage::new(config, &TEST_IDENTITY_A)
            .await
            .unwrap();

        let payment = storage
            .get_payment_by_id("conv-migration-test".to_string())
            .await
            .unwrap();
        let Some(PaymentDetails::Spark {
            conversion_info, ..
        }) = payment.details
        else {
            panic!("Expected Spark payment details");
        };
        match conversion_info.expect("conversion_info should be set") {
            ConversionInfo::Amm {
                pool_id,
                conversion_id,
                status,
                fee,
                ..
            } => {
                assert_eq!(pool_id, "pool-pre");
                assert_eq!(conversion_id, "conv-pre");
                assert_eq!(status, ConversionStatus::Completed);
                assert_eq!(fee, Some(42));
            }
            other => panic!("Expected ConversionInfo::Amm, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_valid_pem_in_storage() {
        let test_ca_pem = generate_test_ca_pem("testca1");
        let result = parse_pem_to_root_store(&test_ca_pem);
        assert!(result.is_ok(), "Expected valid PEM to parse successfully");
        let store = result.unwrap();
        assert_eq!(store.len(), 1, "Expected exactly one certificate in store");
    }

    /// Validates the real `SCHEMA_RENAMES` constant against a hand-rolled
    /// snapshot of the pre-`brz_*` post-multi-tenant schema (i.e. a customer
    /// upgrading from the version of the SDK immediately prior to this PR).
    /// Seeds a payment, runs the SDK's `migrate()` (which fires the rename),
    /// then verifies the renamed schema is functional via the storage trait.
    ///
    /// A typo in `SCHEMA_RENAMES` — wrong table, index, or constraint name —
    /// would fail here either at the rename step or when storage queries
    /// hit a missing identifier.
    #[tokio::test]
    async fn test_rename_against_real_legacy_schema() {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );

        // Seed the legacy (pre-prefix) post-multi-tenant schema directly.
        // Captures the state of a customer at the migration version
        // immediately prior to this PR — `schema_migrations` at version 16,
        // every table user_id-scoped, post-tenant indexes in place.
        let pool = create_pool(&PostgresStorageConfig::with_defaults(
            connection_string.clone(),
        ))
        .expect("create pool");
        let id_lit = format!("'\\x{}'::bytea", hex::encode(TEST_IDENTITY_A));
        {
            let client = pool.get().await.expect("get_conn");
            for stmt in legacy_schema_sql() {
                client
                    .batch_execute(&stmt)
                    .await
                    .unwrap_or_else(|e| panic!("legacy schema setup failed at\n{stmt}\n=> {e}"));
            }
            // Seed: a payment + a setting. Both must survive the rename
            // and be readable via the storage trait after.
            client
                .execute(
                    &format!(
                        "INSERT INTO payments
                         (user_id, id, payment_type, status, amount, fees, timestamp, method)
                         VALUES ({id_lit}, 'p1', 'receive', 'completed', '1000', '0', 100, 'lightning')"
                    ),
                    &[],
                )
                .await
                .expect("seed payment");
            client
                .execute(
                    &format!(
                        "INSERT INTO settings (user_id, key, value)
                         VALUES ({id_lit}, 'seed_key', 'seed_value')"
                    ),
                    &[],
                )
                .await
                .expect("seed setting");
        }

        // Build storage with run_migration=true — fires the rename block,
        // then run_migrations sees schema_migrations at version 16 (now
        // renamed to brz_schema_migrations) and runs only migrations strictly
        // newer than 16 (currently: migration 17, which moves deposit details
        // into the brz_payment_details_deposit table).
        let storage = PostgresStorage::new(
            PostgresStorageConfig::with_defaults(connection_string),
            &TEST_IDENTITY_A,
        )
        .await
        .expect("migrate against legacy schema");

        // Legacy tracker is gone; new one carries the same version row.
        let client = pool.get().await.expect("get_conn");
        let legacy_gone: bool = client
            .query_one(
                "SELECT NOT EXISTS (SELECT 1 FROM information_schema.tables
                                    WHERE table_schema = current_schema()
                                      AND table_name = 'schema_migrations')",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert!(legacy_gone, "legacy schema_migrations must be renamed");

        let version: i32 = client
            .query_one("SELECT MAX(version) FROM brz_schema_migrations", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(version, 18, "migration version must advance to 18");

        // Seed payment row is preserved on the renamed table — proves the
        // table + PK constraint rename worked and the columns line up.
        let payment_row_count: i64 = client
            .query_one("SELECT COUNT(*) FROM brz_payments WHERE id = 'p1'", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(payment_row_count, 1, "seed payment must survive rename");

        // Settings round-trip via the trait — proves brz_settings is wired.
        let setting = storage
            .get_cached_item("seed_key".to_string())
            .await
            .expect("get_cached_item");
        assert_eq!(setting.as_deref(), Some("seed_value"));

        // Write through the trait — proves the post-rename schema accepts
        // new writes via every index/constraint the SDK uses.
        storage
            .set_cached_item(
                "post_rename_key".to_string(),
                "post_rename_value".to_string(),
            )
            .await
            .expect("set_cached_item");
        let written = storage
            .get_cached_item("post_rename_key".to_string())
            .await
            .expect("get_cached_item");
        assert_eq!(written.as_deref(), Some("post_rename_value"));
    }

    /// Hand-rolled snapshot of the pre-PR schema in its terminal
    /// (post-multi-tenant, version-16) state. Maintained alongside
    /// `SCHEMA_RENAMES` — when adding a new table, index, or constraint
    /// to the SDK schema, mirror the addition here so this test continues
    /// to validate that the rename map covers it.
    #[allow(clippy::too_many_lines)]
    fn legacy_schema_sql() -> Vec<String> {
        vec![
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMPTZ DEFAULT NOW()
            )"
            .to_string(),
            "INSERT INTO schema_migrations (version)
             SELECT generate_series(1, 16)"
                .to_string(),
            "CREATE TABLE payments (
                user_id BYTEA NOT NULL,
                id TEXT NOT NULL,
                payment_type TEXT NOT NULL,
                status TEXT NOT NULL,
                amount TEXT NOT NULL,
                fees TEXT NOT NULL,
                timestamp BIGINT NOT NULL,
                method TEXT,
                withdraw_tx_id TEXT,
                deposit_tx_id TEXT,
                spark BOOLEAN,
                PRIMARY KEY (user_id, id)
            )"
            .to_string(),
            "CREATE INDEX idx_payments_user_timestamp ON payments(user_id, timestamp)".to_string(),
            "CREATE INDEX idx_payments_user_payment_type ON payments(user_id, payment_type)"
                .to_string(),
            "CREATE INDEX idx_payments_user_status ON payments(user_id, status)".to_string(),
            "CREATE TABLE settings (
                user_id BYTEA NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (user_id, key)
            )"
            .to_string(),
            "CREATE TABLE unclaimed_deposits (
                user_id BYTEA NOT NULL,
                txid TEXT NOT NULL,
                vout INTEGER NOT NULL,
                amount_sats BIGINT,
                claim_error JSONB,
                refund_tx TEXT,
                refund_tx_id TEXT,
                is_mature BOOLEAN NOT NULL DEFAULT TRUE,
                PRIMARY KEY (user_id, txid, vout)
            )"
            .to_string(),
            "CREATE TABLE payment_metadata (
                user_id BYTEA NOT NULL,
                payment_id TEXT NOT NULL,
                parent_payment_id TEXT,
                lnurl_pay_info JSONB,
                lnurl_withdraw_info JSONB,
                lnurl_description TEXT,
                conversion_info JSONB,
                conversion_status TEXT,
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE INDEX idx_payment_metadata_user_parent
             ON payment_metadata(user_id, parent_payment_id)"
                .to_string(),
            "CREATE TABLE payment_details_lightning (
                user_id BYTEA NOT NULL,
                payment_id TEXT NOT NULL,
                invoice TEXT NOT NULL,
                payment_hash TEXT NOT NULL,
                destination_pubkey TEXT NOT NULL,
                description TEXT,
                htlc_status TEXT NOT NULL DEFAULT 'WaitingForPreimage',
                htlc_expiry_time BIGINT NOT NULL DEFAULT 0,
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE INDEX idx_payment_details_lightning_user_invoice
             ON payment_details_lightning(user_id, invoice)"
                .to_string(),
            "CREATE INDEX idx_payment_details_lightning_user_payment_hash
             ON payment_details_lightning(user_id, payment_hash)"
                .to_string(),
            "CREATE TABLE payment_details_token (
                user_id BYTEA NOT NULL,
                payment_id TEXT NOT NULL,
                metadata JSONB NOT NULL,
                tx_hash TEXT NOT NULL,
                invoice_details JSONB,
                tx_type TEXT NOT NULL DEFAULT 'transfer',
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE TABLE payment_details_spark (
                user_id BYTEA NOT NULL,
                payment_id TEXT NOT NULL,
                invoice_details JSONB,
                htlc_details JSONB,
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE TABLE lnurl_receive_metadata (
                user_id BYTEA NOT NULL,
                payment_hash TEXT NOT NULL,
                nostr_zap_request TEXT,
                nostr_zap_receipt TEXT,
                sender_comment TEXT,
                PRIMARY KEY (user_id, payment_hash)
            )"
            .to_string(),
            "CREATE TABLE sync_revision (
                user_id BYTEA NOT NULL,
                revision BIGINT NOT NULL DEFAULT 0,
                PRIMARY KEY (user_id)
            )"
            .to_string(),
            "CREATE TABLE sync_outgoing (
                user_id BYTEA NOT NULL,
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time BIGINT NOT NULL,
                updated_fields_json JSONB NOT NULL,
                revision BIGINT NOT NULL
            )"
            .to_string(),
            "CREATE INDEX idx_sync_outgoing_user_record_type_data_id
             ON sync_outgoing(user_id, record_type, data_id)"
                .to_string(),
            "CREATE TABLE sync_state (
                user_id BYTEA NOT NULL,
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSONB NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY (user_id, record_type, data_id)
            )"
            .to_string(),
            "CREATE TABLE sync_incoming (
                user_id BYTEA NOT NULL,
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSONB NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY (user_id, record_type, data_id, revision)
            )"
            .to_string(),
            "CREATE INDEX idx_sync_incoming_user_revision
             ON sync_incoming(user_id, revision)"
                .to_string(),
            "CREATE TABLE contacts (
                user_id BYTEA NOT NULL,
                id TEXT NOT NULL,
                name TEXT NOT NULL,
                payment_identifier TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL,
                PRIMARY KEY (user_id, id)
            )"
            .to_string(),
        ]
    }

    /// End-to-end rename against a **pre-multi-tenant** legacy schema
    /// (version=15): unprefixed tables without `user_id`, pre-tenant
    /// indexes only. After `migrate()`, the rename moves pre-tenant indexes
    /// under `brz_*`, then migration 16 (multi-tenant) drops them and
    /// creates `brz_*_user_*` variants. No unprefixed `idx_*` should
    /// survive on any brz_ table.
    #[tokio::test]
    async fn test_rename_against_pre_tenant_legacy_schema() {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );

        let pool = create_pool(&PostgresStorageConfig::with_defaults(
            connection_string.clone(),
        ))
        .expect("create pool");
        {
            let client = pool.get().await.expect("get_conn");
            for stmt in pre_tenant_legacy_schema_sql() {
                client.batch_execute(&stmt).await.unwrap_or_else(|e| {
                    panic!("pre-tenant legacy schema setup failed at\n{stmt}\n=> {e}")
                });
            }
            // Seed data must survive the rename + the multi-tenant
            // migration's user_id backfill.
            client
                .execute(
                    "INSERT INTO payments
                     (id, payment_type, status, amount, fees, timestamp, method)
                     VALUES ('p1', 'receive', 'completed', '1000', '0', 100, 'lightning')",
                    &[],
                )
                .await
                .expect("seed payment");
            client
                .execute(
                    "INSERT INTO settings (key, value) VALUES ('seed_key', 'seed_value')",
                    &[],
                )
                .await
                .expect("seed setting");
        }

        // Build storage with run_migration=true — fires rename then runs
        // migration 16 (the only one left after seed at version=15).
        let storage = PostgresStorage::new(
            PostgresStorageConfig::with_defaults(connection_string),
            &TEST_IDENTITY_A,
        )
        .await
        .expect("migrate against pre-tenant schema");

        let client = pool.get().await.expect("get_conn");

        // The bug check: no unprefixed `idx_*` index should remain on any
        // brz_ table after the rename + multi-tenant migration.
        let orphan_rows = client
            .query(
                "SELECT indexname, tablename FROM pg_indexes
                 WHERE schemaname = current_schema()
                   AND tablename LIKE 'brz_%'
                   AND indexname LIKE 'idx_%'",
                &[],
            )
            .await
            .expect("scan orphan indexes");
        let orphans: Vec<(String, String)> = orphan_rows
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect();
        assert!(
            orphans.is_empty(),
            "found orphan unprefixed indexes after upgrade: {orphans:?}"
        );

        // Migration version advanced from 15 through 18 (16: multi-tenant scope,
        // 17: brz_payment_details_deposit table, 18: conversion_info
        // type-discriminator backfill).
        let version: i32 = client
            .query_one("SELECT MAX(version) FROM brz_schema_migrations", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(version, 18, "migration must advance to 18");

        // Seed data preserved (multi-tenant backfilled user_id to current tenant).
        let payment_count: i64 = client
            .query_one("SELECT COUNT(*) FROM brz_payments WHERE id = 'p1'", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(payment_count, 1, "seed payment must survive upgrade");

        let setting = storage
            .get_cached_item("seed_key".to_string())
            .await
            .expect("get_cached_item");
        assert_eq!(setting.as_deref(), Some("seed_value"));
    }

    /// Pre-multi-tenant schema snapshot (version=15): unprefixed tables
    /// without `user_id`, pre-tenant indexes, no post-tenant indexes.
    /// Captures the SDK schema state just before migration 16.
    #[allow(clippy::too_many_lines)]
    fn pre_tenant_legacy_schema_sql() -> Vec<String> {
        vec![
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMPTZ DEFAULT NOW()
            )"
            .to_string(),
            "INSERT INTO schema_migrations (version)
             SELECT generate_series(1, 15)"
                .to_string(),
            // Core tables (migration 1) — no user_id, simple PKs.
            "CREATE TABLE payments (
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
            )"
            .to_string(),
            "CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )"
            .to_string(),
            "CREATE TABLE unclaimed_deposits (
                txid TEXT NOT NULL,
                vout INTEGER NOT NULL,
                amount_sats BIGINT,
                claim_error JSONB,
                refund_tx TEXT,
                refund_tx_id TEXT,
                is_mature BOOLEAN NOT NULL DEFAULT TRUE,
                PRIMARY KEY (txid, vout)
            )"
            .to_string(),
            "CREATE TABLE payment_metadata (
                payment_id TEXT PRIMARY KEY,
                parent_payment_id TEXT,
                lnurl_pay_info JSONB,
                lnurl_withdraw_info JSONB,
                lnurl_description TEXT,
                conversion_info JSONB,
                conversion_status TEXT
            )"
            .to_string(),
            "CREATE TABLE payment_details_lightning (
                payment_id TEXT PRIMARY KEY,
                invoice TEXT NOT NULL,
                payment_hash TEXT NOT NULL,
                destination_pubkey TEXT NOT NULL,
                description TEXT,
                htlc_status TEXT NOT NULL DEFAULT 'WaitingForPreimage',
                htlc_expiry_time BIGINT NOT NULL DEFAULT 0
            )"
            .to_string(),
            "CREATE TABLE payment_details_token (
                payment_id TEXT PRIMARY KEY,
                metadata JSONB NOT NULL,
                tx_hash TEXT NOT NULL,
                invoice_details JSONB,
                tx_type TEXT NOT NULL DEFAULT 'transfer'
            )"
            .to_string(),
            "CREATE TABLE payment_details_spark (
                payment_id TEXT PRIMARY KEY,
                invoice_details JSONB,
                htlc_details JSONB
            )"
            .to_string(),
            "CREATE TABLE lnurl_receive_metadata (
                payment_hash TEXT PRIMARY KEY,
                nostr_zap_request TEXT,
                nostr_zap_receipt TEXT,
                sender_comment TEXT
            )"
            .to_string(),
            // Sync tables (migration 2) — including pre-tenant indexes.
            "CREATE TABLE sync_revision (
                id INTEGER PRIMARY KEY DEFAULT 1,
                revision BIGINT NOT NULL DEFAULT 0,
                CHECK (id = 1)
            )"
            .to_string(),
            "INSERT INTO sync_revision (id, revision) VALUES (1, 0)".to_string(),
            "CREATE TABLE sync_outgoing (
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time BIGINT NOT NULL,
                updated_fields_json JSONB NOT NULL,
                revision BIGINT NOT NULL
            )"
            .to_string(),
            "CREATE INDEX idx_sync_outgoing_data_id_record_type
             ON sync_outgoing(record_type, data_id)"
                .to_string(),
            "CREATE TABLE sync_state (
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSONB NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY(record_type, data_id)
            )"
            .to_string(),
            "CREATE TABLE sync_incoming (
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSONB NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY(record_type, data_id, revision)
            )"
            .to_string(),
            "CREATE INDEX idx_sync_incoming_revision ON sync_incoming(revision)".to_string(),
            // Pre-tenant indexes on payments etc. (migration 3).
            "CREATE INDEX idx_payments_timestamp ON payments(timestamp)".to_string(),
            "CREATE INDEX idx_payments_payment_type ON payments(payment_type)".to_string(),
            "CREATE INDEX idx_payments_status ON payments(status)".to_string(),
            "CREATE INDEX idx_payment_details_lightning_invoice
             ON payment_details_lightning(invoice)"
                .to_string(),
            "CREATE INDEX idx_payment_metadata_parent
             ON payment_metadata(parent_payment_id)"
                .to_string(),
            // Migration 10 index on payment_hash.
            "CREATE INDEX idx_payment_details_lightning_payment_hash
             ON payment_details_lightning(payment_hash)"
                .to_string(),
            // Migration 11 contacts table.
            "CREATE TABLE contacts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                payment_identifier TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )"
            .to_string(),
        ]
    }
}
