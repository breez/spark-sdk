//! `MySQL`-backed implementation of the `Storage` trait.
//!
//! Direct port of `crates/breez-sdk/core/src/persist/postgres/storage.rs`.
//! See `crates/spark-mysql/src/tree_store.rs` for the SQL syntax translation
//! rules (JSONB→JSON, $N→?, ON CONFLICT→ON DUPLICATE KEY UPDATE, etc.).

use std::collections::HashMap;

use bitcoin::hashes::{Hash, HashEngine, sha256};
use macros::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Params, Pool, Row, Transaction, Value};
use spark_mysql::{mysql_async, tx_opts};
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

use super::base::{Migration, SchemaRenames, map_db_error, run_migrations};
#[cfg(test)]
use super::base::{MysqlStorageConfig, create_pool};

const MIGRATIONS_TABLE: &str = "brz_schema_migrations";
const PAYMENT_UPDATE_LOCK_TIMEOUT_SECS: u64 = 10;

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
        (
            "brz_payments",
            "idx_payments_user_timestamp",
            "brz_idx_payments_user_timestamp",
        ),
        (
            "brz_payments",
            "idx_payments_user_payment_type",
            "brz_idx_payments_user_payment_type",
        ),
        (
            "brz_payments",
            "idx_payments_user_status",
            "brz_idx_payments_user_status",
        ),
        (
            "brz_payment_metadata",
            "idx_payment_metadata_user_parent",
            "brz_idx_payment_metadata_user_parent",
        ),
        (
            "brz_payment_details_lightning",
            "idx_payment_details_lightning_user_invoice",
            "brz_idx_payment_details_lightning_user_invoice",
        ),
        (
            "brz_payment_details_lightning",
            "idx_payment_details_lightning_user_payment_hash",
            "brz_idx_payment_details_lightning_user_payment_hash",
        ),
        (
            "brz_sync_outgoing",
            "idx_sync_outgoing_user_record_type_data_id",
            "brz_idx_sync_outgoing_user_record_type_data_id",
        ),
        (
            "brz_sync_incoming",
            "idx_sync_incoming_user_revision",
            "brz_idx_sync_incoming_user_revision",
        ),
        // Pre-multi-tenant indexes (still present on version < 16 DBs).
        // The multi-tenant migration drops these via the post-rename names.
        (
            "brz_payments",
            "idx_payments_timestamp",
            "brz_idx_payments_timestamp",
        ),
        (
            "brz_payments",
            "idx_payments_payment_type",
            "brz_idx_payments_payment_type",
        ),
        (
            "brz_payments",
            "idx_payments_status",
            "brz_idx_payments_status",
        ),
        (
            "brz_payment_metadata",
            "idx_payment_metadata_parent",
            "brz_idx_payment_metadata_parent",
        ),
        (
            "brz_payment_details_lightning",
            "idx_payment_details_lightning_invoice",
            "brz_idx_payment_details_lightning_invoice",
        ),
        (
            "brz_payment_details_lightning",
            "idx_payment_details_lightning_payment_hash",
            "brz_idx_payment_details_lightning_payment_hash",
        ),
        (
            "brz_sync_outgoing",
            "idx_sync_outgoing_data_id_record_type",
            "brz_idx_sync_outgoing_data_id_record_type",
        ),
        (
            "brz_sync_incoming",
            "idx_sync_incoming_revision",
            "brz_idx_sync_incoming_revision",
        ),
    ],
    foreign_keys: &[],
};

/// `MySQL`-based storage implementation using `mysql_async`'s connection pool.
///
/// Each instance is scoped to a single tenant identity (a 33-byte secp256k1
/// compressed public key). All reads and writes are filtered by `user_id` so
/// that multiple instances with distinct identities can share one `MySQL` DB
/// without seeing each other's data.
pub(crate) struct MysqlStorage {
    pool: Pool,
    /// Tenant identity: 33-byte compressed secp256k1 pubkey. Stored as raw
    /// bytes for direct binding to VARBINARY columns.
    identity: Vec<u8>,
}

impl MysqlStorage {
    #[cfg(test)]
    pub async fn new(config: MysqlStorageConfig, identity: &[u8]) -> Result<Self, StorageError> {
        let run_migration = config.run_migration;
        let pool = create_pool(&config)?;
        Self::new_with_pool(pool, identity, run_migration).await
    }

    /// Creates a new `MysqlStorage` using an existing connection pool.
    ///
    /// Each `MysqlStorage` is scoped to a single tenant `identity`. When
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
    pub(crate) fn migrations(identity: &[u8]) -> Vec<Vec<Migration>> {
        vec![
            // Migration 1: Core tables
            vec![
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_payments (
                        id VARCHAR(255) NOT NULL PRIMARY KEY,
                        payment_type VARCHAR(64) NOT NULL,
                        status VARCHAR(64) NOT NULL,
                        amount VARCHAR(64) NOT NULL,
                        fees VARCHAR(64) NOT NULL,
                        timestamp BIGINT NOT NULL,
                        method VARCHAR(64) NULL,
                        withdraw_tx_id VARCHAR(255) NULL,
                        deposit_tx_id VARCHAR(255) NULL,
                        spark TINYINT(1) NULL
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_settings (
                        `key` VARCHAR(255) NOT NULL PRIMARY KEY,
                        value LONGTEXT NOT NULL
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_unclaimed_deposits (
                        txid VARCHAR(255) NOT NULL,
                        vout INT NOT NULL,
                        amount_sats BIGINT NULL,
                        claim_error JSON NULL,
                        refund_tx LONGTEXT NULL,
                        refund_tx_id VARCHAR(255) NULL,
                        PRIMARY KEY (txid, vout)
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_payment_metadata (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        parent_payment_id VARCHAR(255) NULL,
                        lnurl_pay_info JSON NULL,
                        lnurl_withdraw_info JSON NULL,
                        lnurl_description LONGTEXT NULL,
                        conversion_info JSON NULL
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_payment_details_lightning (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        invoice LONGTEXT NOT NULL,
                        payment_hash VARCHAR(255) NOT NULL,
                        destination_pubkey VARCHAR(255) NOT NULL,
                        description LONGTEXT NULL,
                        preimage VARCHAR(255) NULL
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_payment_details_token (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        metadata JSON NOT NULL,
                        tx_hash VARCHAR(255) NOT NULL,
                        invoice_details JSON NULL
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_payment_details_spark (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        invoice_details JSON NULL,
                        htlc_details JSON NULL
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_lnurl_receive_metadata (
                        payment_hash VARCHAR(255) NOT NULL PRIMARY KEY,
                        nostr_zap_request LONGTEXT NULL,
                        nostr_zap_receipt LONGTEXT NULL,
                        sender_comment LONGTEXT NULL
                    )",
                ),
            ],
            // Migration 2: Sync tables
            vec![
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_sync_revision (
                        id INT NOT NULL PRIMARY KEY DEFAULT 1,
                        revision BIGINT NOT NULL DEFAULT 0,
                        CHECK (id = 1)
                    )",
                ),
                Migration::sql(
                    "INSERT INTO brz_sync_revision (id, revision) VALUES (1, 0)
                     ON DUPLICATE KEY UPDATE id = id",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_sync_outgoing (
                        record_type VARCHAR(255) NOT NULL,
                        data_id VARCHAR(255) NOT NULL,
                        schema_version VARCHAR(64) NOT NULL,
                        commit_time BIGINT NOT NULL,
                        updated_fields_json JSON NOT NULL,
                        revision BIGINT NOT NULL
                    )",
                ),
                Migration::CreateIndex {
                    name: "brz_idx_sync_outgoing_data_id_record_type",
                    table: "brz_sync_outgoing",
                    columns: "(record_type, data_id)",
                },
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_sync_state (
                        record_type VARCHAR(255) NOT NULL,
                        data_id VARCHAR(255) NOT NULL,
                        schema_version VARCHAR(64) NOT NULL,
                        commit_time BIGINT NOT NULL,
                        data JSON NOT NULL,
                        revision BIGINT NOT NULL,
                        PRIMARY KEY(record_type, data_id)
                    )",
                ),
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_sync_incoming (
                        record_type VARCHAR(255) NOT NULL,
                        data_id VARCHAR(255) NOT NULL,
                        schema_version VARCHAR(64) NOT NULL,
                        commit_time BIGINT NOT NULL,
                        data JSON NOT NULL,
                        revision BIGINT NOT NULL,
                        PRIMARY KEY(record_type, data_id, revision)
                    )",
                ),
                Migration::CreateIndex {
                    name: "brz_idx_sync_incoming_revision",
                    table: "brz_sync_incoming",
                    columns: "(revision)",
                },
            ],
            // Migration 3: Indexes
            vec![
                Migration::CreateIndex {
                    name: "brz_idx_payments_timestamp",
                    table: "brz_payments",
                    columns: "(timestamp)",
                },
                Migration::CreateIndex {
                    name: "brz_idx_payments_payment_type",
                    table: "brz_payments",
                    columns: "(payment_type)",
                },
                Migration::CreateIndex {
                    name: "brz_idx_payments_status",
                    table: "brz_payments",
                    columns: "(status)",
                },
                Migration::CreateIndex {
                    name: "brz_idx_payment_details_lightning_invoice",
                    table: "brz_payment_details_lightning",
                    columns: "(invoice(255))",
                },
                Migration::CreateIndex {
                    name: "brz_idx_payment_metadata_parent",
                    table: "brz_payment_metadata",
                    columns: "(parent_payment_id)",
                },
            ],
            // Migration 4: Add tx_type to token payments
            vec![Migration::AddColumn {
                table: "brz_payment_details_token",
                column: "tx_type",
                definition: "VARCHAR(64) NOT NULL DEFAULT 'transfer'",
            }],
            // Migration 5: Clear sync tables to force re-sync
            vec![
                Migration::sql("DELETE FROM brz_sync_outgoing"),
                Migration::sql("DELETE FROM brz_sync_incoming"),
                Migration::sql("DELETE FROM brz_sync_state"),
                Migration::sql("UPDATE brz_sync_revision SET revision = 0"),
                Migration::sql("DELETE FROM brz_settings WHERE `key` = 'sync_initial_complete'"),
            ],
            // Migration 6: Add htlc_status and htlc_expiry_time to lightning payments
            vec![
                Migration::AddColumn {
                    table: "brz_payment_details_lightning",
                    column: "htlc_status",
                    definition: "VARCHAR(64) NOT NULL DEFAULT 'WaitingForPreimage'",
                },
                Migration::AddColumn {
                    table: "brz_payment_details_lightning",
                    column: "htlc_expiry_time",
                    definition: "BIGINT NOT NULL DEFAULT 0",
                },
            ],
            // Migration 7: Backfill htlc_status for existing Lightning payments
            vec![
                Migration::sql(
                    "UPDATE brz_payment_details_lightning
                     SET htlc_status = CASE
                             WHEN (SELECT status FROM brz_payments WHERE id = payment_id) = 'completed' THEN 'PreimageShared'
                             WHEN (SELECT status FROM brz_payments WHERE id = payment_id) = 'pending' THEN 'WaitingForPreimage'
                             ELSE 'Returned'
                         END",
                ),
                Migration::sql(
                    "UPDATE brz_settings
                     SET value = JSON_SET(value, '$.offset', 0)
                     WHERE `key` = 'sync_offset' AND value IS NOT NULL",
                ),
            ],
            // Migration 8: brz_lnurl_receive_metadata preimage column (added then later dropped)
            vec![
                Migration::AddColumn {
                    table: "brz_lnurl_receive_metadata",
                    column: "preimage",
                    definition: "VARCHAR(255) NULL",
                },
                Migration::sql("DELETE FROM brz_settings WHERE `key` = 'lnurl_metadata_updated_after'"),
            ],
            // Migration 9: Clear cached lightning address (schema changed)
            vec![Migration::sql(
                "DELETE FROM brz_settings WHERE `key` = 'lightning_address'",
            )],
            // Migration 10: Index on payment_hash for JOIN with brz_lnurl_receive_metadata
            vec![Migration::CreateIndex {
                name: "brz_idx_payment_details_lightning_payment_hash",
                table: "brz_payment_details_lightning",
                columns: "(payment_hash)",
            }],
            // Migration 11: Contacts table
            vec![Migration::sql(
                "CREATE TABLE IF NOT EXISTS brz_contacts (
                        id VARCHAR(255) NOT NULL PRIMARY KEY,
                        name VARCHAR(255) NOT NULL,
                        payment_identifier VARCHAR(255) NOT NULL,
                        created_at BIGINT NOT NULL,
                        updated_at BIGINT NOT NULL
                    )",
            )],
            // Migration 12: Drop preimage column from brz_lnurl_receive_metadata
            vec![Migration::DropColumn {
                table: "brz_lnurl_receive_metadata",
                column: "preimage",
            }],
            // Migration 13: Clear cached lightning address again (format changed)
            vec![Migration::sql(
                "DELETE FROM brz_settings WHERE `key` = 'lightning_address'",
            )],
            // Migration 14: Add is_mature to brz_unclaimed_deposits
            vec![Migration::AddColumn {
                table: "brz_unclaimed_deposits",
                column: "is_mature",
                definition: "TINYINT(1) NOT NULL DEFAULT 1",
            }],
            // Migration 15: Add conversion_status to brz_payment_metadata
            vec![Migration::AddColumn {
                table: "brz_payment_metadata",
                column: "conversion_status",
                definition: "VARCHAR(64) NULL",
            }],
            // Migration 16: Multi-tenant scoping. Adds `user_id VARBINARY(33)`
            // to every per-user table, backfills it to the current tenant's
            // identity (so existing single-tenant deployments remain readable),
            // sets NOT NULL, and rewrites primary keys / indexes to lead with
            // `user_id`. The literal hex of `identity` is inlined into the SQL
            // backfill: identity bytes come from a typed secp256k1 pubkey so
            // the character set is restricted to `[0-9a-f]{66}` — no
            // SQL-injection surface even though the value is concatenated
            // rather than parameter-bound.
            multi_tenant_migration(identity),
            // Migration 17: Pin the migration-tracking table's `applied_at`
            // default to UTC. The migration manager already writes
            // `UTC_TIMESTAMP(6)` explicitly on INSERT, but aligning the
            // default keeps `SHOW CREATE TABLE` output consistent with the
            // token-store / tree-store migrations table and matches the JS
            // mysql-storage migration of the same name.
            vec![Migration::sql(
                "ALTER TABLE brz_schema_migrations MODIFY COLUMN applied_at \
                 DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))",
            )],
            // Migration 18: Move deposit details into their own table so vout can be
            // NOT NULL and the schema matches brz_payment_details_lightning / _token /
            // _spark. We can't safely backfill the new table from the dropped
            // deposit_tx_id column: we never stored the original SSP output_index,
            // and vout=0 is a valid output index, so defaulting would silently
            // mislabel. Drop the column and leave the brz_payments row in place.
            // The read path sees an unjoined deposit row as `details: None` until
            // the resync re-fetches the SSP user_request and the upsert inserts the
            // new details row.
            vec![
                Migration::sql(
                    "CREATE TABLE IF NOT EXISTS brz_payment_details_deposit (
                        user_id VARBINARY(33) NOT NULL,
                        payment_id VARCHAR(255) NOT NULL,
                        tx_id VARCHAR(255) NOT NULL,
                        vout INT UNSIGNED NOT NULL,
                        PRIMARY KEY (user_id, payment_id)
                    )",
                ),
                Migration::DropColumn {
                    table: "brz_payments",
                    column: "deposit_tx_id",
                },
                Migration::sql(
                    "UPDATE brz_settings
                     SET value = JSON_SET(value, '$.offset', 0)
                     WHERE `key` = 'sync_offset' AND value IS NOT NULL",
                ),
            ],
            // Migration 19: Backfill type discriminator on conversion_info for
            // the ConversionInfo enum refactor. All existing rows are AMM.
            // Mirrors the postgres-side migration. This migration is what
            // covers direct reads from this DB: the `from_json_string_opt`
            // path uses the strict tagged-enum deserializer and would error
            // on an untyped row. (The sync-record entry path is separately
            // covered by `deserialize_conversion_info_with_default_type` on
            // `PaymentMetadata.conversion_info`.)
            vec![Migration::sql(
                "UPDATE brz_payment_metadata \
                 SET conversion_info = JSON_SET(conversion_info, '$.type', 'amm') \
                 WHERE conversion_info IS NOT NULL \
                   AND JSON_EXTRACT(conversion_info, '$.type') IS NULL",
            )],
        ]
    }
}

/// Builds the multi-tenant scoping migration. The `identity` is a 33-byte
/// compressed secp256k1 pubkey; it's hex-encoded and inlined as an `UNHEX(...)`
/// literal so each statement is parameter-free SQL.
#[allow(clippy::too_many_lines)]
fn multi_tenant_migration(identity: &[u8]) -> Vec<Migration> {
    let id_hex = hex::encode(identity);
    // Inline the identity as `UNHEX('...')` — `MySQL` accepts a hex string
    // literal in a binary context, but `UNHEX` is more explicit and works
    // anywhere a `VARBINARY` is expected.
    let id_lit = format!("UNHEX('{id_hex}')");

    let mut stmts: Vec<Migration> = Vec::new();

    let scope_table = |table: &'static str, pk_cols: &str, stmts: &mut Vec<Migration>| {
        stmts.push(Migration::AddColumn {
            // We backfill in a follow-up UPDATE because `MySQL` cannot run
            // `ADD COLUMN ... NOT NULL` without a default for non-empty tables
            // unless we provide a default. The column is added nullable, then
            // populated, then made NOT NULL.
            table,
            column: "user_id",
            definition: "VARBINARY(33) NULL",
        });
        stmts.push(Migration::Sql(format!(
            "UPDATE `{table}` SET user_id = {id_lit} WHERE user_id IS NULL"
        )));
        stmts.push(Migration::Sql(format!(
            "ALTER TABLE `{table}` MODIFY COLUMN user_id VARBINARY(33) NOT NULL"
        )));
        stmts.push(Migration::Sql(format!(
            "ALTER TABLE `{table}` DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, {pk_cols})"
        )));
    };

    scope_table("brz_payments", "id", &mut stmts);
    stmts.push(Migration::DropIndex {
        name: "brz_idx_payments_timestamp",
        table: "brz_payments",
    });
    stmts.push(Migration::DropIndex {
        name: "brz_idx_payments_payment_type",
        table: "brz_payments",
    });
    stmts.push(Migration::DropIndex {
        name: "brz_idx_payments_status",
        table: "brz_payments",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_payments_user_timestamp",
        table: "brz_payments",
        columns: "(user_id, timestamp)",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_payments_user_payment_type",
        table: "brz_payments",
        columns: "(user_id, payment_type)",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_payments_user_status",
        table: "brz_payments",
        columns: "(user_id, status)",
    });

    scope_table("brz_payment_metadata", "payment_id", &mut stmts);
    stmts.push(Migration::DropIndex {
        name: "brz_idx_payment_metadata_parent",
        table: "brz_payment_metadata",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_payment_metadata_user_parent",
        table: "brz_payment_metadata",
        columns: "(user_id, parent_payment_id)",
    });

    scope_table("brz_payment_details_lightning", "payment_id", &mut stmts);
    stmts.push(Migration::DropIndex {
        name: "brz_idx_payment_details_lightning_invoice",
        table: "brz_payment_details_lightning",
    });
    stmts.push(Migration::DropIndex {
        name: "brz_idx_payment_details_lightning_payment_hash",
        table: "brz_payment_details_lightning",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_payment_details_lightning_user_invoice",
        table: "brz_payment_details_lightning",
        columns: "(user_id, invoice(255))",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_payment_details_lightning_user_payment_hash",
        table: "brz_payment_details_lightning",
        columns: "(user_id, payment_hash)",
    });

    scope_table("brz_payment_details_token", "payment_id", &mut stmts);
    scope_table("brz_payment_details_spark", "payment_id", &mut stmts);
    scope_table("brz_lnurl_receive_metadata", "payment_hash", &mut stmts);
    scope_table("brz_unclaimed_deposits", "txid, vout", &mut stmts);
    scope_table("brz_contacts", "id", &mut stmts);
    scope_table("brz_settings", "`key`", &mut stmts);

    // brz_sync_revision was a single-row table (PK id=1, CHECK id=1). Drop the
    // PK and the id column, then re-key by user_id. (`MySQL` 8 auto-drops
    // the dependent CHECK constraint when its sole referenced column goes.)
    stmts.push(Migration::DropPrimaryKey {
        table: "brz_sync_revision",
    });
    stmts.push(Migration::DropColumn {
        table: "brz_sync_revision",
        column: "id",
    });
    stmts.push(Migration::AddColumn {
        table: "brz_sync_revision",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE brz_sync_revision SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE brz_sync_revision MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::sql(
        "ALTER TABLE brz_sync_revision ADD PRIMARY KEY (user_id)",
    ));

    // brz_sync_outgoing has no PK, only an index — just add user_id and rewrite
    // the index.
    stmts.push(Migration::AddColumn {
        table: "brz_sync_outgoing",
        column: "user_id",
        definition: "VARBINARY(33) NULL",
    });
    stmts.push(Migration::Sql(format!(
        "UPDATE brz_sync_outgoing SET user_id = {id_lit} WHERE user_id IS NULL"
    )));
    stmts.push(Migration::sql(
        "ALTER TABLE brz_sync_outgoing MODIFY COLUMN user_id VARBINARY(33) NOT NULL",
    ));
    stmts.push(Migration::DropIndex {
        name: "brz_idx_sync_outgoing_data_id_record_type",
        table: "brz_sync_outgoing",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_sync_outgoing_user_record_type_data_id",
        table: "brz_sync_outgoing",
        columns: "(user_id, record_type, data_id)",
    });

    scope_table("brz_sync_state", "record_type, data_id", &mut stmts);

    scope_table(
        "brz_sync_incoming",
        "record_type, data_id, revision",
        &mut stmts,
    );
    stmts.push(Migration::DropIndex {
        name: "brz_idx_sync_incoming_revision",
        table: "brz_sync_incoming",
    });
    stmts.push(Migration::CreateIndex {
        name: "brz_idx_sync_incoming_user_revision",
        table: "brz_sync_incoming",
        columns: "(user_id, revision)",
    });

    stmts
}

/// Converts an optional serializable value to a JSON string for `JSON` column storage.
fn to_json_string_opt<T: serde::Serialize>(
    value: Option<&T>,
) -> Result<Option<String>, StorageError> {
    value
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Converts an optional JSON string to an optional deserialized type.
fn from_json_string_opt<T: serde::de::DeserializeOwned>(
    value: Option<String>,
) -> Result<Option<T>, StorageError> {
    value
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

impl MysqlStorage {
    fn payment_update_lock_name(identity: &[u8], payment_id: &str) -> String {
        let mut engine = sha256::Hash::engine();
        engine.input(b"brz_payment_update");
        engine.input(identity);
        engine.input(payment_id.as_bytes());
        let digest = sha256::Hash::from_engine(engine);
        digest.to_string()[..32].to_string()
    }

    async fn get_payment_status_in_tx(
        tx: &mut Transaction<'_>,
        identity: &[u8],
        payment_id: &str,
    ) -> Result<Option<PaymentStatus>, StorageError> {
        let status: Option<String> = tx
            .exec_first(
                "SELECT status FROM brz_payments WHERE user_id = ? AND id = ? FOR UPDATE",
                (identity.to_vec(), payment_id),
            )
            .await
            .map_err(map_db_error)?;

        status
            .map(|status| parse_payment_status(&status))
            .transpose()
    }

    #[allow(clippy::too_many_lines)]
    async fn insert_payment_in_tx(
        tx: &mut Transaction<'_>,
        identity: &[u8],
        payment: Payment,
    ) -> Result<(), StorageError> {
        let (withdraw_tx_id, spark): (Option<&str>, Option<bool>) = match &payment.details {
            Some(PaymentDetails::Withdraw { tx_id }) => (Some(tx_id.as_str()), None),
            Some(PaymentDetails::Spark { .. }) => (None, Some(true)),
            _ => (None, None),
        };

        tx.exec_drop(
            "INSERT INTO brz_payments (user_id, id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, spark)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    payment_type = VALUES(payment_type),
                    status = VALUES(status),
                    amount = VALUES(amount),
                    fees = VALUES(fees),
                    timestamp = VALUES(timestamp),
                    method = VALUES(method),
                    withdraw_tx_id = VALUES(withdraw_tx_id),
                    spark = VALUES(spark)",
            (
                identity.to_vec(),
                &payment.id,
                payment.payment_type.to_string(),
                payment.status.to_string(),
                payment.amount.to_string(),
                payment.fees.to_string(),
                i64::try_from(payment.timestamp)?,
                Some(payment.method.to_string()),
                withdraw_tx_id.map(str::to_string),
                spark,
            ),
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
                    let invoice_json = to_json_string_opt(invoice_details.as_ref())?;
                    let htlc_json = to_json_string_opt(htlc_details.as_ref())?;
                    tx.exec_drop(
                        "INSERT INTO brz_payment_details_spark (user_id, payment_id, invoice_details, htlc_details)
                             VALUES (?, ?, ?, ?)
                             ON DUPLICATE KEY UPDATE
                                invoice_details = COALESCE(VALUES(invoice_details), invoice_details),
                                htlc_details = COALESCE(VALUES(htlc_details), htlc_details)",
                        (identity.to_vec(), &payment.id, invoice_json, htlc_json),
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
                let metadata_json = serde_json::to_string(&metadata)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                let invoice_json = to_json_string_opt(invoice_details.as_ref())?;
                tx.exec_drop(
                    "INSERT INTO brz_payment_details_token (user_id, payment_id, metadata, tx_hash, tx_type, invoice_details)
                         VALUES (?, ?, ?, ?, ?, ?)
                         ON DUPLICATE KEY UPDATE
                            metadata = VALUES(metadata),
                            tx_hash = VALUES(tx_hash),
                            tx_type = VALUES(tx_type),
                            invoice_details = COALESCE(VALUES(invoice_details), invoice_details)",
                    (
                        identity.to_vec(),
                        &payment.id,
                        metadata_json,
                        tx_hash,
                        tx_type.to_string(),
                        invoice_json,
                    ),
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
                let payment_hash = htlc_details.payment_hash.clone();
                let preimage = htlc_details.preimage.clone();
                let htlc_status = htlc_details.status.to_string();
                let htlc_expiry_time = i64::try_from(htlc_details.expiry_time)?;
                tx.exec_drop(
                    "INSERT INTO brz_payment_details_lightning (user_id, payment_id, invoice, payment_hash, destination_pubkey, description, preimage, htlc_status, htlc_expiry_time)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                         ON DUPLICATE KEY UPDATE
                            invoice = VALUES(invoice),
                            payment_hash = VALUES(payment_hash),
                            destination_pubkey = VALUES(destination_pubkey),
                            description = VALUES(description),
                            preimage = COALESCE(VALUES(preimage), preimage),
                            htlc_status = COALESCE(VALUES(htlc_status), htlc_status),
                            htlc_expiry_time = COALESCE(VALUES(htlc_expiry_time), htlc_expiry_time)",
                    (
                        identity.to_vec(),
                        &payment.id,
                        invoice,
                        payment_hash,
                        destination_pubkey,
                        description,
                        preimage,
                        htlc_status,
                        htlc_expiry_time,
                    ),
                )
                .await
                .map_err(map_db_error)?;
            }
            Some(PaymentDetails::Deposit { tx_id, vout }) => {
                tx.exec_drop(
                    "INSERT INTO brz_payment_details_deposit (user_id, payment_id, tx_id, vout)
                         VALUES (?, ?, ?, ?)
                         ON DUPLICATE KEY UPDATE
                            tx_id = VALUES(tx_id),
                            vout = VALUES(vout)",
                    (identity.to_vec(), &payment.id, tx_id, vout),
                )
                .await
                .map_err(map_db_error)?;
            }
            Some(PaymentDetails::Withdraw { .. }) | None => {}
        }

        Ok(())
    }
}

#[async_trait]
impl Storage for MysqlStorage {
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    async fn list_payments(
        &self,
        request: StorageListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        // Tenant scoping is always the first WHERE clause; subsequent dynamic
        // filters add more clauses and parameters.
        let mut where_clauses: Vec<String> = vec!["p.user_id = ?".to_string()];
        let mut params: Vec<Value> = vec![Value::from(self.identity.clone())];

        if let Some(ref type_filter) = request.type_filter
            && !type_filter.is_empty()
        {
            let placeholders = build_placeholders(type_filter.len());
            where_clauses.push(format!("p.payment_type IN ({placeholders})"));
            for payment_type in type_filter {
                params.push(Value::from(payment_type.to_string()));
            }
        }

        if let Some(ref status_filter) = request.status_filter
            && !status_filter.is_empty()
        {
            let placeholders = build_placeholders(status_filter.len());
            where_clauses.push(format!("p.status IN ({placeholders})"));
            for status in status_filter {
                params.push(Value::from(status.to_string()));
            }
        }

        if let Some(from_timestamp) = request.from_timestamp {
            where_clauses.push("p.timestamp >= ?".to_string());
            params.push(Value::from(i64::try_from(from_timestamp)?));
        }
        if let Some(to_timestamp) = request.to_timestamp {
            where_clauses.push("p.timestamp < ?".to_string());
            params.push(Value::from(i64::try_from(to_timestamp)?));
        }

        if let Some(ref asset_filter) = request.asset_filter {
            match asset_filter {
                AssetFilter::Bitcoin => {
                    where_clauses.push("t.metadata IS NULL".to_string());
                }
                AssetFilter::Token { token_identifier } => {
                    where_clauses.push("t.metadata IS NOT NULL".to_string());
                    if let Some(identifier) = token_identifier {
                        where_clauses.push(
                            "JSON_UNQUOTE(JSON_EXTRACT(t.metadata, '$.identifier')) = ?"
                                .to_string(),
                        );
                        params.push(Value::from(identifier.clone()));
                    }
                }
            }
        }

        if let Some(ref payment_details_filter) = request.payment_details_filter {
            let mut all_payment_details_clauses = Vec::new();
            for payment_details_filter in payment_details_filter {
                let mut payment_details_clauses = Vec::new();
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
                    let placeholders = build_placeholders(htlc_statuses.len());
                    if alias == "l" {
                        payment_details_clauses.push(format!("l.htlc_status IN ({placeholders})"));
                    } else {
                        payment_details_clauses.push(format!(
                            "JSON_UNQUOTE(JSON_EXTRACT(s.htlc_details, '$.status')) IN ({placeholders})"
                        ));
                    }
                    for htlc_status in htlc_statuses {
                        params.push(Value::from(htlc_status.to_string()));
                    }
                }
                // Payment type discriminator: brz_payments.spark is `true` for
                // Spark transfers and `NULL` for token transactions.
                match payment_details_filter {
                    StoragePaymentDetailsFilter::Spark { .. } => {
                        payment_details_clauses.push("p.spark = true".to_string());
                    }
                    StoragePaymentDetailsFilter::Token { .. } => {
                        payment_details_clauses.push("p.spark IS NULL".to_string());
                    }
                    StoragePaymentDetailsFilter::Lightning { .. } => {}
                }

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
                            "JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, '$.type')) = 'amm' \
                             AND JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, '$.status')) \
                             = 'RefundNeeded'"
                        }
                        crate::persist::ConversionFilter::OrchestraPending => {
                            "JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, '$.type')) \
                             = 'orchestra' AND JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, \
                             '$.status')) NOT IN ('Completed', 'Failed', 'Refunded')"
                        }
                        crate::persist::ConversionFilter::BoltzPending => {
                            "JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, '$.type')) \
                             = 'boltz' AND JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, \
                             '$.status')) NOT IN ('Completed', 'Failed', 'Refunded')"
                        }
                    };
                    payment_details_clauses.push(format!(
                        "pm.conversion_info IS NOT NULL AND {status_clause}"
                    ));
                }
                if let StoragePaymentDetailsFilter::Token {
                    tx_hash: Some(tx_hash),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push("t.tx_hash = ?".to_string());
                    params.push(Value::from(tx_hash.clone()));
                }
                if let StoragePaymentDetailsFilter::Token {
                    tx_type: Some(tx_type),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push("t.tx_type = ?".to_string());
                    params.push(Value::from(tx_type.to_string()));
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

        where_clauses.push("pm.parent_payment_id IS NULL".to_string());

        // Build the WHERE clause (always non-empty: tenant scoping is the first clause).
        let where_sql = format!("WHERE {}", where_clauses.join(" AND "));

        let order_direction = if request.sort_ascending.unwrap_or(false) {
            "ASC"
        } else {
            "DESC"
        };

        let limit = i64::from(request.limit.unwrap_or(u32::MAX));
        let offset = i64::from(request.offset.unwrap_or(0));

        let query = format!(
            "{SELECT_PAYMENT_SQL} {where_sql} ORDER BY p.timestamp {order_direction} LIMIT ? OFFSET ?"
        );

        params.push(Value::from(limit));
        params.push(Value::from(offset));

        let rows: Vec<Row> = conn
            .exec(&query, Params::Positional(params))
            .await
            .map_err(map_db_error)?;

        let mut payments = Vec::new();
        for row in &rows {
            payments.push(map_payment(row)?);
        }
        Ok(payments)
    }

    async fn apply_payment_update(&self, payment: Payment) -> Result<bool, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let lock_name = Self::payment_update_lock_name(&self.identity, &payment.id);
        let acquired: Option<i64> = conn
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (lock_name.as_str(), PAYMENT_UPDATE_LOCK_TIMEOUT_SECS),
            )
            .await
            .map_err(map_db_error)?;
        if acquired != Some(1) {
            return Err(StorageError::Implementation(format!(
                "Timed out acquiring payment update lock '{lock_name}'"
            )));
        }

        let result = async {
            let mut tx = conn
                .start_transaction(tx_opts())
                .await
                .map_err(map_db_error)?;
            let stored_status =
                Self::get_payment_status_in_tx(&mut tx, &self.identity, &payment.id).await?;

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
                    payment.id, payment.status
                );
            }
            Self::insert_payment_in_tx(&mut tx, &self.identity, payment).await?;
            tx.commit().await.map_err(map_db_error)?;
            Ok(!same_status)
        }
        .await;

        let release_result = conn
            .exec_drop("SELECT RELEASE_LOCK(?)", (lock_name.as_str(),))
            .await
            .map_err(map_db_error);

        match (result, release_result) {
            (Ok(should_emit), Ok(())) => Ok(should_emit),
            (Err(e), _) | (Ok(_), Err(e)) => Err(e),
        }
    }

    async fn insert_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let lnurl_pay_info_json = to_json_string_opt(metadata.lnurl_pay_info.as_ref())?;
        let lnurl_withdraw_info_json = to_json_string_opt(metadata.lnurl_withdraw_info.as_ref())?;
        let conversion_info_json = to_json_string_opt(metadata.conversion_info.as_ref())?;
        let conversion_status_str = metadata
            .conversion_status
            .as_ref()
            .map(std::string::ToString::to_string);

        conn.exec_drop(
            "INSERT INTO brz_payment_metadata (user_id, payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info, conversion_status)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                parent_payment_id = COALESCE(VALUES(parent_payment_id), parent_payment_id),
                lnurl_pay_info = COALESCE(VALUES(lnurl_pay_info), lnurl_pay_info),
                lnurl_withdraw_info = COALESCE(VALUES(lnurl_withdraw_info), lnurl_withdraw_info),
                lnurl_description = COALESCE(VALUES(lnurl_description), lnurl_description),
                conversion_info = COALESCE(VALUES(conversion_info), conversion_info),
                conversion_status = COALESCE(VALUES(conversion_status), conversion_status)",
            (
                self.identity.clone(),
                payment_id,
                metadata.parent_payment_id,
                lnurl_pay_info_json,
                lnurl_withdraw_info_json,
                metadata.lnurl_description,
                conversion_info_json,
                conversion_status_str,
            ),
        )
        .await
        .map_err(map_db_error)?;

        Ok(())
    }

    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        conn.exec_drop(
            "INSERT INTO brz_settings (user_id, `key`, value) VALUES (?, ?, ?)
             ON DUPLICATE KEY UPDATE value = VALUES(value)",
            (self.identity.clone(), key, value),
        )
        .await
        .map_err(map_db_error)?;

        Ok(())
    }

    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let row: Option<String> = conn
            .exec_first(
                "SELECT value FROM brz_settings WHERE user_id = ? AND `key` = ?",
                (self.identity.clone(), key),
            )
            .await
            .map_err(map_db_error)?;

        Ok(row)
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        conn.exec_drop(
            "DELETE FROM brz_settings WHERE user_id = ? AND `key` = ?",
            (self.identity.clone(), key),
        )
        .await
        .map_err(map_db_error)?;

        Ok(())
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.user_id = ? AND p.id = ?");
        let row: Option<Row> = conn
            .exec_first(&query, (self.identity.clone(), id))
            .await
            .map_err(map_db_error)?;
        let row = row.ok_or(StorageError::NotFound)?;
        map_payment(&row)
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.user_id = ? AND l.invoice = ?");
        let row: Option<Row> = conn
            .exec_first(&query, (self.identity.clone(), invoice))
            .await
            .map_err(map_db_error)?;

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

        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let has_related: bool = conn
            .exec_first::<i64, _, _>(
                "SELECT EXISTS(SELECT 1 FROM brz_payment_metadata WHERE user_id = ? AND parent_payment_id IS NOT NULL LIMIT 1)",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_db_error)?
            .is_some_and(|v| v != 0);

        if !has_related {
            return Ok(HashMap::new());
        }

        let placeholders = build_placeholders(parent_payment_ids.len());
        let query = format!(
            "{SELECT_PAYMENT_SQL} WHERE p.user_id = ? AND pm.parent_payment_id IN ({placeholders}) ORDER BY p.timestamp ASC"
        );

        let mut params: Vec<Value> = vec![Value::from(self.identity.clone())];
        params.extend(parent_payment_ids.iter().cloned().map(Value::from));

        let rows: Vec<Row> = conn
            .exec(&query, Params::Positional(params))
            .await
            .map_err(map_db_error)?;

        let mut result: HashMap<String, Vec<Payment>> = HashMap::new();
        for row in &rows {
            let payment = map_payment(row)?;
            let parent_payment_id: String = row
                .get(32)
                .ok_or_else(|| StorageError::Implementation("missing parent_payment_id".into()))?;
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
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        conn.exec_drop(
            "INSERT INTO brz_unclaimed_deposits (user_id, txid, vout, amount_sats, is_mature)
             VALUES (?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE is_mature = VALUES(is_mature), amount_sats = VALUES(amount_sats)",
            (
                self.identity.clone(),
                txid,
                i32::try_from(vout)?,
                i64::try_from(amount_sats)?,
                is_mature,
            ),
        )
        .await
        .map_err(map_db_error)?;
        Ok(())
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        conn.exec_drop(
            "DELETE FROM brz_unclaimed_deposits WHERE user_id = ? AND txid = ? AND vout = ?",
            (self.identity.clone(), txid, i32::try_from(vout)?),
        )
        .await
        .map_err(map_db_error)?;
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let rows: Vec<Row> = conn
            .exec(
                "SELECT txid, vout, amount_sats, is_mature, claim_error, refund_tx, refund_tx_id FROM brz_unclaimed_deposits WHERE user_id = ?",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_db_error)?;

        let mut deposits = Vec::new();
        for row in &rows {
            let claim_error_str: Option<String> = get_opt_str(row, 4);
            let claim_error: Option<DepositClaimError> = from_json_string_opt(claim_error_str)?;

            deposits.push(DepositInfo {
                txid: get_str(row, 0)?,
                vout: u32::try_from(
                    row.get::<Option<i32>, _>(1)
                        .ok_or_else(|| StorageError::Implementation("missing vout".into()))?
                        .ok_or_else(|| StorageError::Implementation("vout is NULL".into()))?,
                )?,
                amount_sats: get_opt_i64(row, 2)
                    .map(u64::try_from)
                    .transpose()?
                    .unwrap_or(0),
                is_mature: get_opt_bool(row, 3)
                    .ok_or_else(|| StorageError::Implementation("is_mature is NULL".into()))?,
                claim_error,
                refund_tx: get_opt_str(row, 5),
                refund_tx_id: get_opt_str(row, 6),
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
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        match payload {
            UpdateDepositPayload::ClaimError { error } => {
                let error_json = serde_json::to_string(&error)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                conn.exec_drop(
                    "UPDATE brz_unclaimed_deposits SET claim_error = ?, refund_tx = NULL, refund_tx_id = NULL WHERE user_id = ? AND txid = ? AND vout = ?",
                    (error_json, self.identity.clone(), txid, i32::try_from(vout)?),
                )
                .await
                .map_err(map_db_error)?;
            }
            UpdateDepositPayload::Refund {
                refund_txid,
                refund_tx,
            } => {
                conn.exec_drop(
                    "UPDATE brz_unclaimed_deposits SET refund_tx = ?, refund_tx_id = ?, claim_error = NULL WHERE user_id = ? AND txid = ? AND vout = ?",
                    (refund_tx, refund_txid, self.identity.clone(), txid, i32::try_from(vout)?),
                )
                .await
                .map_err(map_db_error)?;
            }
        }
        Ok(())
    }

    async fn set_lnurl_metadata(
        &self,
        metadata: Vec<SetLnurlMetadataItem>,
    ) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        for m in metadata {
            conn.exec_drop(
                "INSERT INTO brz_lnurl_receive_metadata (user_id, payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
                 VALUES (?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    nostr_zap_request = VALUES(nostr_zap_request),
                    nostr_zap_receipt = VALUES(nostr_zap_receipt),
                    sender_comment = VALUES(sender_comment)",
                (self.identity.clone(), m.payment_hash, m.nostr_zap_request, m.nostr_zap_receipt, m.sender_comment),
            )
            .await
            .map_err(map_db_error)?;
        }
        Ok(())
    }

    async fn list_contacts(
        &self,
        request: ListContactsRequest,
    ) -> Result<Vec<Contact>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let limit = i64::from(request.limit.unwrap_or(u32::MAX));
        let offset = i64::from(request.offset.unwrap_or(0));

        let rows: Vec<(String, String, String, i64, i64)> = conn
            .exec(
                "SELECT id, name, payment_identifier, created_at, updated_at
                 FROM brz_contacts WHERE user_id = ? ORDER BY name ASC LIMIT ? OFFSET ?",
                (self.identity.clone(), limit, offset),
            )
            .await
            .map_err(map_db_error)?;

        let mut contacts = Vec::new();
        for (id, name, payment_identifier, created_at, updated_at) in rows {
            contacts.push(Contact {
                id,
                name,
                payment_identifier,
                created_at: u64::try_from(created_at)?,
                updated_at: u64::try_from(updated_at)?,
            });
        }
        Ok(contacts)
    }

    async fn get_contact(&self, id: String) -> Result<Contact, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let row: Option<(String, String, String, i64, i64)> = conn
            .exec_first(
                "SELECT id, name, payment_identifier, created_at, updated_at
                 FROM brz_contacts WHERE user_id = ? AND id = ?",
                (self.identity.clone(), id),
            )
            .await
            .map_err(map_db_error)?;
        let (id, name, payment_identifier, created_at, updated_at) =
            row.ok_or(StorageError::NotFound)?;
        Ok(Contact {
            id,
            name,
            payment_identifier,
            created_at: u64::try_from(created_at)?,
            updated_at: u64::try_from(updated_at)?,
        })
    }

    async fn insert_contact(&self, contact: Contact) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        conn.exec_drop(
            "INSERT INTO brz_contacts (user_id, id, name, payment_identifier, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
               name = VALUES(name),
               payment_identifier = VALUES(payment_identifier),
               updated_at = VALUES(updated_at)",
            (
                self.identity.clone(),
                contact.id,
                contact.name,
                contact.payment_identifier,
                i64::try_from(contact.created_at)?,
                i64::try_from(contact.updated_at)?,
            ),
        )
        .await
        .map_err(map_db_error)?;
        Ok(())
    }

    async fn delete_contact(&self, id: String) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        conn.exec_drop(
            "DELETE FROM brz_contacts WHERE user_id = ? AND id = ?",
            (self.identity.clone(), id),
        )
        .await
        .map_err(map_db_error)?;
        Ok(())
    }

    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let mut tx = conn
            .start_transaction(tx_opts())
            .await
            .map_err(map_db_error)?;

        // The local queue revision is per-tenant — two tenants don't share a queue.
        let local_revision: i64 = tx
            .exec_first(
                "SELECT COALESCE(MAX(revision), 0) + 1 FROM brz_sync_outgoing WHERE user_id = ?",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_db_error)?
            .unwrap_or(1);

        let updated_fields_json = serde_json::to_string(&record.updated_fields)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.exec_drop(
            "INSERT INTO brz_sync_outgoing (user_id, record_type, data_id, schema_version, commit_time, updated_fields_json, revision)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                self.identity.clone(),
                record.id.r#type,
                record.id.data_id,
                record.schema_version,
                commit_time,
                updated_fields_json,
                local_revision,
            ),
        )
        .await
        .map_err(map_db_error)?;

        tx.commit().await.map_err(map_db_error)?;

        Ok(u64::try_from(local_revision)?)
    }

    async fn complete_outgoing_sync(
        &self,
        record: Record,
        local_revision: u64,
    ) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let mut tx = conn
            .start_transaction(tx_opts())
            .await
            .map_err(map_db_error)?;

        let mut result = tx
            .exec_iter(
                "DELETE FROM brz_sync_outgoing WHERE user_id = ? AND record_type = ? AND data_id = ? AND revision = ?",
                (
                    self.identity.clone(),
                    record.id.r#type.clone(),
                    record.id.data_id.clone(),
                    i64::try_from(local_revision)?,
                ),
            )
            .await
            .map_err(map_db_error)?;
        let rows_deleted = result.affected_rows();
        let _: Vec<Row> = result.collect().await.map_err(map_db_error)?;

        if rows_deleted == 0 {
            warn!(
                "complete_outgoing_sync: DELETE from brz_sync_outgoing matched 0 rows \
                 (type={}, data_id={}, revision={})",
                record.id.r#type, record.id.data_id, local_revision
            );
        }

        let data_json = serde_json::to_string(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.exec_drop(
            "INSERT INTO brz_sync_state (user_id, record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    schema_version = VALUES(schema_version),
                    commit_time = VALUES(commit_time),
                    data = VALUES(data),
                    revision = VALUES(revision)",
            (
                self.identity.clone(),
                record.id.r#type,
                record.id.data_id,
                record.schema_version,
                commit_time,
                data_json,
                i64::try_from(record.revision)?,
            ),
        )
        .await
        .map_err(map_db_error)?;

        // Upsert this tenant's revision row. Migration 16 created a row at
        // backfill, but a fresh tenant joining a shared DB after the migration
        // won't have one yet.
        tx.exec_drop(
            "INSERT INTO brz_sync_revision (user_id, revision) VALUES (?, ?)
             ON DUPLICATE KEY UPDATE revision = GREATEST(revision, VALUES(revision))",
            (self.identity.clone(), i64::try_from(record.revision)?),
        )
        .await
        .map_err(map_db_error)?;

        tx.commit().await.map_err(map_db_error)?;

        Ok(())
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let rows: Vec<Row> = conn
            .exec(
                "SELECT o.record_type, o.data_id, o.schema_version, o.commit_time, o.updated_fields_json, o.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM brz_sync_outgoing o
                 LEFT JOIN brz_sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id AND o.user_id = e.user_id
                 WHERE o.user_id = ?
                 ORDER BY o.revision ASC
                 LIMIT ?",
                (self.identity.clone(), i64::from(limit)),
            )
            .await
            .map_err(map_db_error)?;

        let mut results = Vec::new();
        for row in &rows {
            let existing_data: Option<String> = get_opt_str(row, 8);
            let parent = if let Some(existing_data) = existing_data {
                Some(Record {
                    id: RecordId::new(get_str(row, 0)?, get_str(row, 1)?),
                    schema_version: get_str(row, 6)?,
                    revision: u64::try_from(get_i64(row, 9)?)?,
                    data: serde_json::from_str(&existing_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                })
            } else {
                None
            };
            let updated_fields_str: String = get_str(row, 4)?;
            let change = RecordChange {
                id: RecordId::new(get_str(row, 0)?, get_str(row, 1)?),
                schema_version: get_str(row, 2)?,
                updated_fields: serde_json::from_str(&updated_fields_str)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                local_revision: u64::try_from(get_i64(row, 5)?)?,
            };
            results.push(OutgoingChange { change, parent });
        }

        Ok(results)
    }

    async fn get_last_revision(&self) -> Result<u64, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        // A tenant that hasn't synced anything yet may have no row; treat as 0.
        let revision: i64 = conn
            .exec_first(
                "SELECT revision FROM brz_sync_revision WHERE user_id = ?",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_db_error)?
            .unwrap_or(0);

        Ok(u64::try_from(revision)?)
    }

    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError> {
        if records.is_empty() {
            return Ok(());
        }

        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let commit_time = chrono::Utc::now().timestamp();

        for record in records {
            let data_json = serde_json::to_string(&record.data)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            conn.exec_drop(
                "INSERT INTO brz_sync_incoming (user_id, record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    schema_version = VALUES(schema_version),
                    commit_time = VALUES(commit_time),
                    data = VALUES(data)",
                (
                    self.identity.clone(),
                    record.id.r#type,
                    record.id.data_id,
                    record.schema_version,
                    commit_time,
                    data_json,
                    i64::try_from(record.revision)?,
                ),
            )
            .await
            .map_err(map_db_error)?;
        }

        Ok(())
    }

    async fn delete_incoming_record(&self, record: Record) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        conn.exec_drop(
            "DELETE FROM brz_sync_incoming WHERE user_id = ? AND record_type = ? AND data_id = ? AND revision = ?",
            (
                self.identity.clone(),
                record.id.r#type,
                record.id.data_id,
                i64::try_from(record.revision)?,
            ),
        )
        .await
        .map_err(map_db_error)?;

        Ok(())
    }

    async fn get_incoming_records(&self, limit: u32) -> Result<Vec<IncomingChange>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let rows: Vec<Row> = conn
            .exec(
                "SELECT i.record_type, i.data_id, i.schema_version, i.data, i.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM brz_sync_incoming i
                 LEFT JOIN brz_sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id AND i.user_id = e.user_id
                 WHERE i.user_id = ?
                 ORDER BY i.revision ASC
                 LIMIT ?",
                (self.identity.clone(), i64::from(limit)),
            )
            .await
            .map_err(map_db_error)?;

        let mut results = Vec::new();
        for row in &rows {
            let existing_data: Option<String> = get_opt_str(row, 7);
            let old_state = if let Some(existing_data) = existing_data {
                Some(Record {
                    id: RecordId::new(get_str(row, 0)?, get_str(row, 1)?),
                    schema_version: get_str(row, 5)?,
                    revision: u64::try_from(get_i64(row, 8)?)?,
                    data: serde_json::from_str(&existing_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                })
            } else {
                None
            };
            let data_str: String = get_str(row, 3)?;
            let new_state = Record {
                id: RecordId::new(get_str(row, 0)?, get_str(row, 1)?),
                schema_version: get_str(row, 2)?,
                data: serde_json::from_str(&data_str)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                revision: u64::try_from(get_i64(row, 4)?)?,
            };
            results.push(IncomingChange {
                new_state,
                old_state,
            });
        }

        Ok(results)
    }

    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let row: Option<Row> = conn
            .exec_first(
                "SELECT o.record_type, o.data_id, o.schema_version, o.commit_time, o.updated_fields_json, o.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM brz_sync_outgoing o
                 LEFT JOIN brz_sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id AND o.user_id = e.user_id
                 WHERE o.user_id = ?
                 ORDER BY o.revision DESC
                 LIMIT 1",
                (self.identity.clone(),),
            )
            .await
            .map_err(map_db_error)?;

        if let Some(row) = row {
            let existing_data: Option<String> = get_opt_str(&row, 8);
            let parent = if let Some(existing_data) = existing_data {
                Some(Record {
                    id: RecordId::new(get_str(&row, 0)?, get_str(&row, 1)?),
                    schema_version: get_str(&row, 6)?,
                    revision: u64::try_from(get_i64(&row, 9)?)?,
                    data: serde_json::from_str(&existing_data)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?,
                })
            } else {
                None
            };
            let updated_fields_str: String = get_str(&row, 4)?;
            let change = RecordChange {
                id: RecordId::new(get_str(&row, 0)?, get_str(&row, 1)?),
                schema_version: get_str(&row, 2)?,
                updated_fields: serde_json::from_str(&updated_fields_str)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                local_revision: u64::try_from(get_i64(&row, 5)?)?,
            };
            return Ok(Some(OutgoingChange { change, parent }));
        }

        Ok(None)
    }

    async fn update_record_from_incoming(&self, record: Record) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let mut tx = conn
            .start_transaction(tx_opts())
            .await
            .map_err(map_db_error)?;

        let data_json = serde_json::to_string(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.exec_drop(
            "INSERT INTO brz_sync_state (user_id, record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    schema_version = VALUES(schema_version),
                    commit_time = VALUES(commit_time),
                    data = VALUES(data),
                    revision = VALUES(revision)",
            (
                self.identity.clone(),
                record.id.r#type,
                record.id.data_id,
                record.schema_version,
                commit_time,
                data_json,
                i64::try_from(record.revision)?,
            ),
        )
        .await
        .map_err(map_db_error)?;

        tx.exec_drop(
            "INSERT INTO brz_sync_revision (user_id, revision) VALUES (?, ?)
             ON DUPLICATE KEY UPDATE revision = GREATEST(revision, VALUES(revision))",
            (self.identity.clone(), i64::try_from(record.revision)?),
        )
        .await
        .map_err(map_db_error)?;

        tx.commit().await.map_err(map_db_error)?;

        Ok(())
    }
}

/// Base query for payment lookups. Indices 0-31 are used by `map_payment`,
/// index 32 (`parent_payment_id`) is only used by `get_payments_by_parent_ids`.
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
      LEFT JOIN brz_payment_details_deposit pd ON p.id = pd.payment_id AND p.user_id = pd.user_id
      LEFT JOIN brz_payment_details_token t ON p.id = t.payment_id AND p.user_id = t.user_id
      LEFT JOIN brz_payment_details_spark s ON p.id = s.payment_id AND p.user_id = s.user_id
      LEFT JOIN brz_payment_metadata pm ON p.id = pm.payment_id AND p.user_id = pm.user_id
      LEFT JOIN brz_lnurl_receive_metadata lrm ON l.payment_hash = lrm.payment_hash AND l.user_id = lrm.user_id";

#[allow(clippy::too_many_lines)]
fn map_payment(row: &Row) -> Result<Payment, StorageError> {
    let withdraw_tx_id: Option<String> = get_opt_str(row, 7);
    let deposit_tx_id: Option<String> = get_opt_str(row, 8);
    let spark: Option<bool> = get_opt_bool(row, 10);
    let lightning_invoice: Option<String> = get_opt_str(row, 11);
    let token_metadata: Option<String> = get_opt_str(row, 21);

    let details = match (
        lightning_invoice,
        withdraw_tx_id,
        deposit_tx_id,
        spark,
        token_metadata,
    ) {
        (Some(invoice), _, _, _, _) => {
            let payment_hash: String = get_str(row, 12)?;
            let destination_pubkey: String = get_str(row, 13)?;
            let description: Option<String> = get_opt_str(row, 14);
            let preimage: Option<String> = get_opt_str(row, 15);
            let htlc_status_str: Option<String> = get_opt_str(row, 16);
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
            let htlc_expiry_time: i64 = get_i64(row, 17)?;
            let htlc_details = SparkHtlcDetails {
                payment_hash,
                preimage,
                expiry_time: u64::try_from(htlc_expiry_time)?,
                status: htlc_status,
            };
            let lnurl_pay_info_str: Option<String> = get_opt_str(row, 18);
            let lnurl_withdraw_info_str: Option<String> = get_opt_str(row, 19);
            let lnurl_nostr_zap_request: Option<String> = get_opt_str(row, 27);
            let lnurl_nostr_zap_receipt: Option<String> = get_opt_str(row, 28);
            let lnurl_sender_comment: Option<String> = get_opt_str(row, 29);
            let lnurl_payment_hash: Option<String> = get_opt_str(row, 30);

            let lnurl_pay_info: Option<LnurlPayInfo> = from_json_string_opt(lnurl_pay_info_str)?;
            let lnurl_withdraw_info: Option<LnurlWithdrawInfo> =
                from_json_string_opt(lnurl_withdraw_info_str)?;
            let conversion_info_str: Option<String> = get_opt_str(row, 20);
            let conversion_info: Option<ConversionInfo> =
                from_json_string_opt(conversion_info_str)?;

            let lnurl_receive_metadata = if lnurl_payment_hash.is_some() {
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
        (_, _, Some(tx_id), _, _) => Some(PaymentDetails::Deposit {
            tx_id,
            vout: get_opt_u32(row, 9).ok_or_else(|| {
                StorageError::Serialization("deposit row missing deposit_vout".to_string())
            })?,
        }),
        (_, _, _, Some(_), _) => {
            let invoice_details_str: Option<String> = get_opt_str(row, 25);
            let invoice_details = from_json_string_opt(invoice_details_str)?;
            let htlc_details_str: Option<String> = get_opt_str(row, 26);
            let htlc_details = from_json_string_opt(htlc_details_str)?;
            let conversion_info_str: Option<String> = get_opt_str(row, 20);
            let conversion_info: Option<ConversionInfo> =
                from_json_string_opt(conversion_info_str)?;
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                conversion_info,
            })
        }
        (_, _, _, _, Some(metadata_str)) => {
            let tx_type_str: String = get_str(row, 23)?;
            let tx_type = tx_type_str
                .parse()
                .map_err(|e: String| StorageError::Serialization(e))?;
            let invoice_details_str: Option<String> = get_opt_str(row, 24);
            let invoice_details = from_json_string_opt(invoice_details_str)?;
            let conversion_info_str: Option<String> = get_opt_str(row, 20);
            let conversion_info: Option<ConversionInfo> =
                from_json_string_opt(conversion_info_str)?;
            Some(PaymentDetails::Token {
                metadata: serde_json::from_str(&metadata_str)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                tx_hash: get_str(row, 22)?,
                tx_type,
                invoice_details,
                conversion_info,
            })
        }
        _ => None,
    };

    let payment_type_str: String = get_str(row, 1)?;
    let status_str: String = get_str(row, 2)?;
    let amount_str: String = get_str(row, 3)?;
    let fees_str: String = get_str(row, 4)?;
    let method_str: Option<String> = get_opt_str(row, 6);

    Ok(Payment {
        id: get_str(row, 0)?,
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
        timestamp: u64::try_from(get_i64(row, 5)?)?,
        details,
        method: method_str.map_or(PaymentMethod::Lightning, |s| {
            s.trim_matches('"')
                .to_lowercase()
                .parse()
                .unwrap_or(PaymentMethod::Lightning)
        }),
        conversion_details: {
            let conversion_status_str: Option<String> = get_opt_str(row, 31);
            conversion_status_str
                .map(|s| {
                    s.parse::<ConversionStatus>()
                        .map(|status| ConversionDetails {
                            status,
                            conversions: Vec::new(),
                        })
                        .map_err(StorageError::Serialization)
                })
                .transpose()?
        },
    })
}

fn build_placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n.saturating_mul(3));
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        s.push('?');
    }
    s
}

fn get_str(row: &Row, idx: usize) -> Result<String, StorageError> {
    // `Row::get::<T, _>(idx)` panics during conversion when T is non-Option
    // and the column value is NULL. Read as `Option<String>` first so NULL
    // surfaces as `Some(None)` and a missing column as `None`, then collapse
    // both into the same "missing" error path.
    row.get::<Option<String>, _>(idx)
        .ok_or_else(|| StorageError::Implementation(format!("missing column at index {idx}")))?
        .ok_or_else(|| StorageError::Implementation(format!("column at index {idx} is NULL")))
}

fn get_i64(row: &Row, idx: usize) -> Result<i64, StorageError> {
    row.get::<Option<i64>, _>(idx)
        .ok_or_else(|| StorageError::Implementation(format!("missing i64 column at index {idx}")))?
        .ok_or_else(|| StorageError::Implementation(format!("i64 column at index {idx} is NULL")))
}

/// NULL-safe `row.get` for nullable columns. `Row::get::<T, _>(idx)` panics on
/// NULL during `FromValue` conversion when `T` is non-`Option`; reading as
/// `Option<T>` and flattening avoids the panic and treats both "column
/// missing" and "value NULL" as `None`.
fn get_opt_str(row: &Row, idx: usize) -> Option<String> {
    row.get::<Option<String>, _>(idx).flatten()
}

fn get_opt_bool(row: &Row, idx: usize) -> Option<bool> {
    row.get::<Option<bool>, _>(idx).flatten()
}

fn get_opt_i64(row: &Row, idx: usize) -> Option<i64> {
    row.get::<Option<i64>, _>(idx).flatten()
}

fn get_opt_u32(row: &Row, idx: usize) -> Option<u32> {
    row.get::<Option<u32>, _>(idx).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

    /// A fixed 33-byte test identity used by single-tenant test fixtures.
    /// Two-tenant isolation tests use a different identity for the second tenant.
    pub(super) const TEST_IDENTITY_A: [u8; 33] = [
        0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0x20,
    ];

    struct MysqlTestFixture {
        storage: MysqlStorage,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl MysqlTestFixture {
        async fn new() -> Self {
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container");

            let host_port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get host port");

            let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

            let storage = MysqlStorage::new(
                MysqlStorageConfig::with_defaults(connection_string),
                &TEST_IDENTITY_A,
            )
            .await
            .expect("Failed to create MysqlStorage");

            Self { storage, container }
        }
    }

    #[tokio::test]
    async fn test_mysql_storage() {
        let fixture = MysqlTestFixture::new().await;
        Box::pin(crate::persist::tests::test_storage(Box::new(
            fixture.storage,
        )))
        .await;
    }

    #[tokio::test]
    async fn test_unclaimed_deposits_crud() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_unclaimed_deposits_crud(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_deposit_refunds() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_deposit_refunds(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_type_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_type_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_status_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_status_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_metadata() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_metadata(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_sync_storage() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_sync_storage(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_contacts_crud() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_contacts_crud(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_asset_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_asset_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_timestamp_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_timestamp_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_spark_htlc_status_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_spark_htlc_status_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_lightning_htlc_details_and_status_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_lightning_htlc_details_and_status_filtering(Box::new(
            fixture.storage,
        ))
        .await;
    }

    #[tokio::test]
    async fn test_conversion_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_conversion_filtering(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_token_transaction_type_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_token_transaction_type_filtering(Box::new(fixture.storage))
            .await;
    }

    #[tokio::test]
    async fn test_combined_filters() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_combined_filters(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_sort_order() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_sort_order(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_payment_details_update_persistence() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_details_update_persistence(Box::new(fixture.storage))
            .await;
    }

    #[tokio::test]
    async fn test_payment_terminal_status_is_not_replaced() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_terminal_status_is_not_replaced(Box::new(
            fixture.storage,
        ))
        .await;
    }

    #[tokio::test]
    async fn test_payment_metadata_merge() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_metadata_merge(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_conversion_status_persistence() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_conversion_status_persistence(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_insert_boltz_conversion_info() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_insert_boltz_conversion_info(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_update_boltz_status_to_completed() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_update_boltz_status_to_completed(Box::new(fixture.storage))
            .await;
    }

    /// Migration backfill: an untyped (pre-migration) AMM `conversion_info`
    /// row is upgraded to a tagged enum and reads back via the strict
    /// `from_json_string_opt::<ConversionInfo>` path that `list_payments` /
    /// `get_payment_by_id` use.
    #[tokio::test]
    async fn test_migration_conversion_info_type_discriminator() {
        use crate::{ConversionInfo, ConversionStatus, PaymentDetails, Storage};

        let container = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");
        let host_port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");
        let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

        // Apply all migrations except the discriminator backfill (the last one).
        let migrations = MysqlStorage::migrations(&TEST_IDENTITY_A);
        let backfill_index = migrations.len() - 1;
        let pool = create_pool(&MysqlStorageConfig::with_defaults(
            connection_string.clone(),
        ))
        .expect("create pool");
        run_migrations(
            &pool,
            MIGRATIONS_TABLE,
            &migrations[..backfill_index],
            Some(&SCHEMA_RENAMES),
        )
        .await
        .expect("partial migrations");

        // Insert a Spark payment + an untyped (pre-migration) AMM row.
        let id_lit = format!("UNHEX('{}')", hex::encode(TEST_IDENTITY_A));
        let untyped = serde_json::json!({
            "pool_id": "pool-pre",
            "conversion_id": "conv-pre",
            "status": "Completed",
            "fee": "42",
            "purpose": null,
        });
        {
            let mut conn = pool.get_conn().await.expect("get_conn");
            conn.query_drop(format!(
                "INSERT INTO brz_payments
                 (user_id, id, payment_type, status, amount, fees, timestamp, method, spark)
                 VALUES ({id_lit}, 'conv-migration-test', 'send', 'completed', '5000', '10', \
                  1700000001, '\"spark\"', 1)"
            ))
            .await
            .expect("insert payment");
            conn.query_drop(format!(
                "INSERT INTO brz_payment_details_spark (user_id, payment_id) \
                 VALUES ({id_lit}, 'conv-migration-test')"
            ))
            .await
            .expect("insert payment_details_spark");
            conn.exec_drop(
                format!(
                    "INSERT INTO brz_payment_metadata (user_id, payment_id, conversion_info) \
                     VALUES ({id_lit}, 'conv-migration-test', ?)"
                ),
                (untyped.to_string(),),
            )
            .await
            .expect("insert payment_metadata");
        }

        // Open via MysqlStorage to trigger the remaining backfill migration.
        let storage = MysqlStorage::new(
            MysqlStorageConfig::with_defaults(connection_string),
            &TEST_IDENTITY_A,
        )
        .await
        .expect("MysqlStorage::new (runs backfill)");

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

    /// Simulates the post-migration state for a legacy deposit: a row exists in
    /// `brz_payments` with `method = 'deposit'` but no matching
    /// `brz_payment_details_deposit` row (the SSP `user_request` hasn't been
    /// re-fetched yet). `list_payments` must return the payment with
    /// `details: None` and `method: Deposit` preserved.
    #[tokio::test]
    async fn test_legacy_deposit_without_details_row_returns_none() {
        use crate::PaymentMethod;
        use crate::persist::StorageListPaymentsRequest;
        use mysql_async::prelude::Queryable;

        let fixture = MysqlTestFixture::new().await;

        // Insert a deposit brz_payments row directly via the pool, bypassing
        // insert_payment_in_tx so no brz_payment_details_deposit row is written.
        let mut conn = fixture.storage.pool.get_conn().await.expect("get_conn");
        conn.exec_drop(
            "INSERT INTO brz_payments
             (user_id, id, payment_type, status, amount, fees, timestamp, method)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            (
                TEST_IDENTITY_A.to_vec(),
                "legacy-deposit-1",
                "receive",
                "completed",
                "1000",
                "0",
                1_000_i64,
                "deposit",
            ),
        )
        .await
        .expect("seed legacy deposit");
        drop(conn);

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

    /// Two `MysqlStorage` instances with distinct identities sharing one
    /// connection pool / DB. The container must be kept alive for the test.
    struct TwoTenantFixture {
        a: MysqlStorage,
        b: MysqlStorage,
        #[allow(dead_code)]
        container: ContainerAsync<Mysql>,
    }

    impl TwoTenantFixture {
        async fn new() -> Self {
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container");

            let host_port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get host port");

            let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

            let config = MysqlStorageConfig::with_defaults(connection_string);
            let pool = create_pool(&config).expect("Failed to create pool");

            let a = MysqlStorage::new_with_pool(pool.clone(), &TEST_IDENTITY_A, true)
                .await
                .expect("Failed to create tenant A");
            let b = MysqlStorage::new_with_pool(pool, &TEST_IDENTITY_B, true)
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
        let b_pending = fx.b.get_pending_outgoing_changes(100).await.unwrap();
        assert!(
            b_pending.is_empty(),
            "tenant B must not see tenant A's pending outgoing"
        );
        assert_eq!(fx.b.get_last_revision().await.unwrap(), 0);

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
        fx.b.delete_incoming_record(rec_b.clone()).await.unwrap();
        let a_incoming = fx.a.get_incoming_records(100).await.unwrap();
        assert_eq!(
            a_incoming.len(),
            1,
            "tenant A's incoming must survive B's delete on the same key"
        );

        let list_b_final =
            fx.b.list_payments(StorageListPaymentsRequest::default())
                .await
                .unwrap();
        assert_eq!(list_b_final.len(), 1);
        assert_eq!(list_b_final[0].id, "pmt_shared_id");
    }

    /// `MySQL` counterpart of the Postgres `test_rename_against_real_legacy_schema`.
    /// Validates the real `SCHEMA_RENAMES` constant against a hand-rolled
    /// snapshot of the pre-`brz_*` post-multi-tenant schema. A typo in the
    /// rename map (table, index, or FK name) would fail here.
    #[tokio::test]
    async fn test_rename_against_real_legacy_schema() {
        let container = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");
        let host_port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");
        let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

        let pool = create_pool(&MysqlStorageConfig::with_defaults(
            connection_string.clone(),
        ))
        .expect("create pool");
        let id_lit = format!("UNHEX('{}')", hex::encode(TEST_IDENTITY_A));
        {
            let mut conn = pool.get_conn().await.expect("get_conn");
            for stmt in legacy_schema_sql() {
                conn.query_drop(&stmt)
                    .await
                    .unwrap_or_else(|e| panic!("legacy schema setup failed at\n{stmt}\n=> {e}"));
            }
            conn.query_drop(format!(
                "INSERT INTO payments
                 (user_id, id, payment_type, status, amount, fees, timestamp, method)
                 VALUES ({id_lit}, 'p1', 'receive', 'completed', '1000', '0', 100, 'lightning')"
            ))
            .await
            .expect("seed payment");
            conn.query_drop(format!(
                "INSERT INTO settings (user_id, `key`, value)
                 VALUES ({id_lit}, 'seed_key', 'seed_value')"
            ))
            .await
            .expect("seed setting");
        }

        let storage = MysqlStorage::new(
            MysqlStorageConfig::with_defaults(connection_string),
            &TEST_IDENTITY_A,
        )
        .await
        .expect("migrate against legacy schema");

        let mut conn = pool.get_conn().await.expect("get_conn");

        let legacy_count: Option<i64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.tables
                 WHERE table_schema = DATABASE() AND table_name = 'schema_migrations'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            legacy_count,
            Some(0),
            "legacy schema_migrations must be renamed"
        );

        let version: Option<i32> = conn
            .exec_first("SELECT MAX(version) FROM brz_schema_migrations", ())
            .await
            .unwrap();
        assert_eq!(
            version,
            Some(19),
            "migration version must advance to 19 (the legacy fixture starts at 16; \
             migration 17 pins applied_at default to UTC; migration 18 moves deposit \
             details into the brz_payment_details_deposit table; migration 19 backfills \
             the conversion_info type discriminator)"
        );

        let payment_count: Option<i64> = conn
            .exec_first("SELECT COUNT(*) FROM brz_payments WHERE id = 'p1'", ())
            .await
            .unwrap();
        assert_eq!(payment_count, Some(1), "seed payment must survive rename");

        let setting = storage
            .get_cached_item("seed_key".to_string())
            .await
            .expect("get_cached_item");
        assert_eq!(setting.as_deref(), Some("seed_value"));

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

    /// Hand-rolled snapshot of the pre-PR `MySQL` schema in its terminal
    /// (post-multi-tenant, version-16) state. Mirror additions here when
    /// extending the SDK schema, so this test continues to validate that
    /// `SCHEMA_RENAMES` covers every new identifier.
    #[allow(clippy::too_many_lines)]
    fn legacy_schema_sql() -> Vec<String> {
        vec![
            "CREATE TABLE schema_migrations (
                version INT PRIMARY KEY,
                applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
            )"
            .to_string(),
            (1..=16)
                .map(|v| format!("INSERT INTO schema_migrations (version) VALUES ({v})"))
                .collect::<Vec<_>>()
                .join(";"),
            "CREATE TABLE payments (
                user_id VARBINARY(33) NOT NULL,
                id VARCHAR(255) NOT NULL,
                payment_type VARCHAR(64) NOT NULL,
                status VARCHAR(64) NOT NULL,
                amount VARCHAR(64) NOT NULL,
                fees VARCHAR(64) NOT NULL,
                timestamp BIGINT NOT NULL,
                method VARCHAR(64) NULL,
                withdraw_tx_id VARCHAR(255) NULL,
                deposit_tx_id VARCHAR(255) NULL,
                spark TINYINT(1) NULL,
                PRIMARY KEY (user_id, id)
            )"
            .to_string(),
            "CREATE INDEX idx_payments_user_timestamp ON payments(user_id, timestamp)".to_string(),
            "CREATE INDEX idx_payments_user_payment_type ON payments(user_id, payment_type)"
                .to_string(),
            "CREATE INDEX idx_payments_user_status ON payments(user_id, status)".to_string(),
            "CREATE TABLE settings (
                user_id VARBINARY(33) NOT NULL,
                `key` VARCHAR(255) NOT NULL,
                value LONGTEXT NOT NULL,
                PRIMARY KEY (user_id, `key`)
            )"
            .to_string(),
            "CREATE TABLE unclaimed_deposits (
                user_id VARBINARY(33) NOT NULL,
                txid VARCHAR(255) NOT NULL,
                vout INT NOT NULL,
                amount_sats BIGINT NULL,
                claim_error JSON NULL,
                refund_tx LONGTEXT NULL,
                refund_tx_id VARCHAR(255) NULL,
                is_mature TINYINT(1) NOT NULL DEFAULT 1,
                PRIMARY KEY (user_id, txid, vout)
            )"
            .to_string(),
            "CREATE TABLE payment_metadata (
                user_id VARBINARY(33) NOT NULL,
                payment_id VARCHAR(255) NOT NULL,
                parent_payment_id VARCHAR(255) NULL,
                lnurl_pay_info JSON NULL,
                lnurl_withdraw_info JSON NULL,
                lnurl_description LONGTEXT NULL,
                conversion_info JSON NULL,
                conversion_status VARCHAR(64) NULL,
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE INDEX idx_payment_metadata_user_parent
             ON payment_metadata(user_id, parent_payment_id)"
                .to_string(),
            "CREATE TABLE payment_details_lightning (
                user_id VARBINARY(33) NOT NULL,
                payment_id VARCHAR(255) NOT NULL,
                invoice LONGTEXT NOT NULL,
                payment_hash VARCHAR(255) NOT NULL,
                destination_pubkey VARCHAR(255) NOT NULL,
                description LONGTEXT NULL,
                htlc_status VARCHAR(64) NOT NULL DEFAULT 'WaitingForPreimage',
                htlc_expiry_time BIGINT NOT NULL DEFAULT 0,
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE INDEX idx_payment_details_lightning_user_invoice
             ON payment_details_lightning(user_id, invoice(255))"
                .to_string(),
            "CREATE INDEX idx_payment_details_lightning_user_payment_hash
             ON payment_details_lightning(user_id, payment_hash)"
                .to_string(),
            "CREATE TABLE payment_details_token (
                user_id VARBINARY(33) NOT NULL,
                payment_id VARCHAR(255) NOT NULL,
                metadata JSON NOT NULL,
                tx_hash VARCHAR(255) NOT NULL,
                invoice_details JSON NULL,
                tx_type VARCHAR(64) NOT NULL DEFAULT 'transfer',
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE TABLE payment_details_spark (
                user_id VARBINARY(33) NOT NULL,
                payment_id VARCHAR(255) NOT NULL,
                invoice_details JSON NULL,
                htlc_details JSON NULL,
                PRIMARY KEY (user_id, payment_id)
            )"
            .to_string(),
            "CREATE TABLE lnurl_receive_metadata (
                user_id VARBINARY(33) NOT NULL,
                payment_hash VARCHAR(255) NOT NULL,
                nostr_zap_request LONGTEXT NULL,
                nostr_zap_receipt LONGTEXT NULL,
                sender_comment LONGTEXT NULL,
                PRIMARY KEY (user_id, payment_hash)
            )"
            .to_string(),
            "CREATE TABLE sync_revision (
                user_id VARBINARY(33) NOT NULL,
                revision BIGINT NOT NULL DEFAULT 0,
                PRIMARY KEY (user_id)
            )"
            .to_string(),
            "CREATE TABLE sync_outgoing (
                user_id VARBINARY(33) NOT NULL,
                record_type VARCHAR(255) NOT NULL,
                data_id VARCHAR(255) NOT NULL,
                schema_version VARCHAR(64) NOT NULL,
                commit_time BIGINT NOT NULL,
                updated_fields_json JSON NOT NULL,
                revision BIGINT NOT NULL
            )"
            .to_string(),
            "CREATE INDEX idx_sync_outgoing_user_record_type_data_id
             ON sync_outgoing(user_id, record_type, data_id)"
                .to_string(),
            "CREATE TABLE sync_state (
                user_id VARBINARY(33) NOT NULL,
                record_type VARCHAR(255) NOT NULL,
                data_id VARCHAR(255) NOT NULL,
                schema_version VARCHAR(64) NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSON NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY (user_id, record_type, data_id)
            )"
            .to_string(),
            "CREATE TABLE sync_incoming (
                user_id VARBINARY(33) NOT NULL,
                record_type VARCHAR(255) NOT NULL,
                data_id VARCHAR(255) NOT NULL,
                schema_version VARCHAR(64) NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSON NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY (user_id, record_type, data_id, revision)
            )"
            .to_string(),
            "CREATE INDEX idx_sync_incoming_user_revision
             ON sync_incoming(user_id, revision)"
                .to_string(),
            "CREATE TABLE contacts (
                user_id VARBINARY(33) NOT NULL,
                id VARCHAR(255) NOT NULL,
                name VARCHAR(255) NOT NULL,
                payment_identifier VARCHAR(255) NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL,
                PRIMARY KEY (user_id, id)
            )"
            .to_string(),
        ]
    }

    /// `MySQL` counterpart of the Postgres
    /// `test_rename_against_pre_tenant_legacy_schema`. Validates that the
    /// rename + multi-tenant migration leaves no orphan unprefixed indexes
    /// on any brz_ table.
    #[tokio::test]
    async fn test_rename_against_pre_tenant_legacy_schema() {
        let container = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");
        let host_port = container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");
        let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

        let pool = create_pool(&MysqlStorageConfig::with_defaults(
            connection_string.clone(),
        ))
        .expect("create pool");
        {
            let mut conn = pool.get_conn().await.expect("get_conn");
            for stmt in pre_tenant_legacy_schema_sql() {
                conn.query_drop(&stmt).await.unwrap_or_else(|e| {
                    panic!("pre-tenant legacy schema setup failed at\n{stmt}\n=> {e}")
                });
            }
            conn.query_drop(
                "INSERT INTO payments
                 (id, payment_type, status, amount, fees, timestamp, method)
                 VALUES ('p1', 'receive', 'completed', '1000', '0', 100, 'lightning')",
            )
            .await
            .expect("seed payment");
            conn.query_drop(
                "INSERT INTO settings (`key`, value) VALUES ('seed_key', 'seed_value')",
            )
            .await
            .expect("seed setting");
        }

        let storage = MysqlStorage::new(
            MysqlStorageConfig::with_defaults(connection_string),
            &TEST_IDENTITY_A,
        )
        .await
        .expect("migrate against pre-tenant schema");

        let mut conn = pool.get_conn().await.expect("get_conn");

        // The bug check: no unprefixed `idx_*` index on any brz_ table.
        // `information_schema.statistics` lists one row per (table, index,
        // column) — DISTINCT collapses to unique (table, index) pairs.
        let orphan_rows: Vec<(String, String)> = conn
            .exec(
                "SELECT DISTINCT table_name, index_name
                 FROM information_schema.statistics
                 WHERE table_schema = DATABASE()
                   AND table_name LIKE 'brz\\_%'
                   AND index_name LIKE 'idx\\_%'",
                (),
            )
            .await
            .expect("scan orphan indexes");
        assert!(
            orphan_rows.is_empty(),
            "found orphan unprefixed indexes after upgrade: {orphan_rows:?}"
        );

        let version: Option<i32> = conn
            .exec_first("SELECT MAX(version) FROM brz_schema_migrations", ())
            .await
            .unwrap();
        assert_eq!(version, Some(19), "migration must advance to 19");

        let payment_count: Option<i64> = conn
            .exec_first("SELECT COUNT(*) FROM brz_payments WHERE id = 'p1'", ())
            .await
            .unwrap();
        assert_eq!(payment_count, Some(1), "seed payment must survive upgrade");

        let setting = storage
            .get_cached_item("seed_key".to_string())
            .await
            .expect("get_cached_item");
        assert_eq!(setting.as_deref(), Some("seed_value"));
    }

    /// Pre-multi-tenant `MySQL` schema snapshot (version=15).
    #[allow(clippy::too_many_lines)]
    fn pre_tenant_legacy_schema_sql() -> Vec<String> {
        vec![
            "CREATE TABLE schema_migrations (
                version INT PRIMARY KEY,
                applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
            )"
            .to_string(),
            (1..=15)
                .map(|v| format!("INSERT INTO schema_migrations (version) VALUES ({v})"))
                .collect::<Vec<_>>()
                .join(";"),
            "CREATE TABLE payments (
                id VARCHAR(255) NOT NULL PRIMARY KEY,
                payment_type VARCHAR(64) NOT NULL,
                status VARCHAR(64) NOT NULL,
                amount VARCHAR(64) NOT NULL,
                fees VARCHAR(64) NOT NULL,
                timestamp BIGINT NOT NULL,
                method VARCHAR(64) NULL,
                withdraw_tx_id VARCHAR(255) NULL,
                deposit_tx_id VARCHAR(255) NULL,
                spark TINYINT(1) NULL
            )"
            .to_string(),
            "CREATE TABLE settings (
                `key` VARCHAR(255) NOT NULL PRIMARY KEY,
                value LONGTEXT NOT NULL
            )"
            .to_string(),
            "CREATE TABLE unclaimed_deposits (
                txid VARCHAR(255) NOT NULL,
                vout INT NOT NULL,
                amount_sats BIGINT NULL,
                claim_error JSON NULL,
                refund_tx LONGTEXT NULL,
                refund_tx_id VARCHAR(255) NULL,
                is_mature TINYINT(1) NOT NULL DEFAULT 1,
                PRIMARY KEY (txid, vout)
            )"
            .to_string(),
            "CREATE TABLE payment_metadata (
                payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                parent_payment_id VARCHAR(255) NULL,
                lnurl_pay_info JSON NULL,
                lnurl_withdraw_info JSON NULL,
                lnurl_description LONGTEXT NULL,
                conversion_info JSON NULL,
                conversion_status VARCHAR(64) NULL
            )"
            .to_string(),
            "CREATE TABLE payment_details_lightning (
                payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                invoice LONGTEXT NOT NULL,
                payment_hash VARCHAR(255) NOT NULL,
                destination_pubkey VARCHAR(255) NOT NULL,
                description LONGTEXT NULL,
                htlc_status VARCHAR(64) NOT NULL DEFAULT 'WaitingForPreimage',
                htlc_expiry_time BIGINT NOT NULL DEFAULT 0
            )"
            .to_string(),
            "CREATE TABLE payment_details_token (
                payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                metadata JSON NOT NULL,
                tx_hash VARCHAR(255) NOT NULL,
                invoice_details JSON NULL,
                tx_type VARCHAR(64) NOT NULL DEFAULT 'transfer'
            )"
            .to_string(),
            "CREATE TABLE payment_details_spark (
                payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                invoice_details JSON NULL,
                htlc_details JSON NULL
            )"
            .to_string(),
            "CREATE TABLE lnurl_receive_metadata (
                payment_hash VARCHAR(255) NOT NULL PRIMARY KEY,
                nostr_zap_request LONGTEXT NULL,
                nostr_zap_receipt LONGTEXT NULL,
                sender_comment LONGTEXT NULL
            )"
            .to_string(),
            "CREATE TABLE sync_revision (
                id INT NOT NULL PRIMARY KEY DEFAULT 1,
                revision BIGINT NOT NULL DEFAULT 0,
                CHECK (id = 1)
            )"
            .to_string(),
            "INSERT INTO sync_revision (id, revision) VALUES (1, 0)".to_string(),
            "CREATE TABLE sync_outgoing (
                record_type VARCHAR(255) NOT NULL,
                data_id VARCHAR(255) NOT NULL,
                schema_version VARCHAR(64) NOT NULL,
                commit_time BIGINT NOT NULL,
                updated_fields_json JSON NOT NULL,
                revision BIGINT NOT NULL
            )"
            .to_string(),
            "CREATE INDEX idx_sync_outgoing_data_id_record_type
             ON sync_outgoing(record_type, data_id)"
                .to_string(),
            "CREATE TABLE sync_state (
                record_type VARCHAR(255) NOT NULL,
                data_id VARCHAR(255) NOT NULL,
                schema_version VARCHAR(64) NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSON NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY(record_type, data_id)
            )"
            .to_string(),
            "CREATE TABLE sync_incoming (
                record_type VARCHAR(255) NOT NULL,
                data_id VARCHAR(255) NOT NULL,
                schema_version VARCHAR(64) NOT NULL,
                commit_time BIGINT NOT NULL,
                data JSON NOT NULL,
                revision BIGINT NOT NULL,
                PRIMARY KEY(record_type, data_id, revision)
            )"
            .to_string(),
            "CREATE INDEX idx_sync_incoming_revision ON sync_incoming(revision)".to_string(),
            "CREATE INDEX idx_payments_timestamp ON payments(timestamp)".to_string(),
            "CREATE INDEX idx_payments_payment_type ON payments(payment_type)".to_string(),
            "CREATE INDEX idx_payments_status ON payments(status)".to_string(),
            "CREATE INDEX idx_payment_details_lightning_invoice
             ON payment_details_lightning(invoice(255))"
                .to_string(),
            "CREATE INDEX idx_payment_metadata_parent
             ON payment_metadata(parent_payment_id)"
                .to_string(),
            "CREATE INDEX idx_payment_details_lightning_payment_hash
             ON payment_details_lightning(payment_hash)"
                .to_string(),
            "CREATE TABLE contacts (
                id VARCHAR(255) NOT NULL PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                payment_identifier VARCHAR(255) NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )"
            .to_string(),
        ]
    }
}
