//! `MySQL`-backed implementation of the `Storage` trait.
//!
//! Direct port of `crates/breez-sdk/core/src/persist/postgres/storage.rs`.
//! See `crates/spark-mysql/src/tree_store.rs` for the SQL syntax translation
//! rules (JSONB→JSON, $N→?, ON CONFLICT→ON DUPLICATE KEY UPDATE, etc.).

use std::collections::HashMap;

use macros::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Params, Pool, Row, TxOpts, Value};
use spark_mysql::mysql_async;
use tracing::warn;

use crate::{
    AssetFilter, Contact, ConversionDetails, ConversionInfo, ConversionStatus, DepositInfo,
    ListContactsRequest, LnurlPayInfo, LnurlReceiveMetadata, LnurlWithdrawInfo, PaymentDetails,
    PaymentMethod, SparkHtlcDetails, SparkHtlcStatus,
    error::DepositClaimError,
    persist::{
        Payment, PaymentMetadata, SetLnurlMetadataItem, Storage, StorageError,
        StorageListPaymentsRequest, StoragePaymentDetailsFilter, UpdateDepositPayload,
    },
    sync_storage::{
        IncomingChange, OutgoingChange, Record, RecordChange, RecordId, UnversionedRecordChange,
    },
};

use super::base::{Migration, map_db_error, run_migrations};
#[cfg(test)]
use super::base::{MysqlStorageConfig, create_pool};

const MIGRATIONS_TABLE: &str = "schema_migrations";

/// `MySQL`-based storage implementation using `mysql_async`'s connection pool.
pub(crate) struct MysqlStorage {
    pool: Pool,
}

impl MysqlStorage {
    #[cfg(test)]
    pub async fn new(config: MysqlStorageConfig) -> Result<Self, StorageError> {
        let pool = create_pool(&config)?;
        Self::new_with_pool(pool).await
    }

    pub async fn new_with_pool(pool: Pool) -> Result<Self, StorageError> {
        let storage = Self { pool };
        storage.migrate().await?;
        Ok(storage)
    }

    async fn migrate(&self) -> Result<(), StorageError> {
        run_migrations(&self.pool, MIGRATIONS_TABLE, &Self::migrations()).await
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn migrations() -> Vec<&'static [Migration]> {
        vec![
            // Migration 1: Core tables
            &[
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS payments (
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
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS settings (
                        `key` VARCHAR(255) NOT NULL PRIMARY KEY,
                        value LONGTEXT NOT NULL
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS unclaimed_deposits (
                        txid VARCHAR(255) NOT NULL,
                        vout INT NOT NULL,
                        amount_sats BIGINT NULL,
                        claim_error JSON NULL,
                        refund_tx LONGTEXT NULL,
                        refund_tx_id VARCHAR(255) NULL,
                        PRIMARY KEY (txid, vout)
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS payment_metadata (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        parent_payment_id VARCHAR(255) NULL,
                        lnurl_pay_info JSON NULL,
                        lnurl_withdraw_info JSON NULL,
                        lnurl_description LONGTEXT NULL,
                        conversion_info JSON NULL
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS payment_details_lightning (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        invoice LONGTEXT NOT NULL,
                        payment_hash VARCHAR(255) NOT NULL,
                        destination_pubkey VARCHAR(255) NOT NULL,
                        description LONGTEXT NULL,
                        preimage VARCHAR(255) NULL
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS payment_details_token (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        metadata JSON NOT NULL,
                        tx_hash VARCHAR(255) NOT NULL,
                        invoice_details JSON NULL
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS payment_details_spark (
                        payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
                        invoice_details JSON NULL,
                        htlc_details JSON NULL
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS lnurl_receive_metadata (
                        payment_hash VARCHAR(255) NOT NULL PRIMARY KEY,
                        nostr_zap_request LONGTEXT NULL,
                        nostr_zap_receipt LONGTEXT NULL,
                        sender_comment LONGTEXT NULL
                    )",
                ),
            ],
            // Migration 2: Sync tables
            &[
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS sync_revision (
                        id INT NOT NULL PRIMARY KEY DEFAULT 1,
                        revision BIGINT NOT NULL DEFAULT 0,
                        CHECK (id = 1)
                    )",
                ),
                Migration::Sql(
                    "INSERT INTO sync_revision (id, revision) VALUES (1, 0)
                     ON DUPLICATE KEY UPDATE id = id",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS sync_outgoing (
                        record_type VARCHAR(255) NOT NULL,
                        data_id VARCHAR(255) NOT NULL,
                        schema_version VARCHAR(64) NOT NULL,
                        commit_time BIGINT NOT NULL,
                        updated_fields_json JSON NOT NULL,
                        revision BIGINT NOT NULL
                    )",
                ),
                Migration::CreateIndex {
                    name: "idx_sync_outgoing_data_id_record_type",
                    table: "sync_outgoing",
                    columns: "(record_type, data_id)",
                },
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS sync_state (
                        record_type VARCHAR(255) NOT NULL,
                        data_id VARCHAR(255) NOT NULL,
                        schema_version VARCHAR(64) NOT NULL,
                        commit_time BIGINT NOT NULL,
                        data JSON NOT NULL,
                        revision BIGINT NOT NULL,
                        PRIMARY KEY(record_type, data_id)
                    )",
                ),
                Migration::Sql(
                    "CREATE TABLE IF NOT EXISTS sync_incoming (
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
                    name: "idx_sync_incoming_revision",
                    table: "sync_incoming",
                    columns: "(revision)",
                },
            ],
            // Migration 3: Indexes
            &[
                Migration::CreateIndex {
                    name: "idx_payments_timestamp",
                    table: "payments",
                    columns: "(timestamp)",
                },
                Migration::CreateIndex {
                    name: "idx_payments_payment_type",
                    table: "payments",
                    columns: "(payment_type)",
                },
                Migration::CreateIndex {
                    name: "idx_payments_status",
                    table: "payments",
                    columns: "(status)",
                },
                Migration::CreateIndex {
                    name: "idx_payment_details_lightning_invoice",
                    table: "payment_details_lightning",
                    columns: "(invoice(255))",
                },
                Migration::CreateIndex {
                    name: "idx_payment_metadata_parent",
                    table: "payment_metadata",
                    columns: "(parent_payment_id)",
                },
            ],
            // Migration 4: Add tx_type to token payments
            &[Migration::AddColumn {
                table: "payment_details_token",
                column: "tx_type",
                definition: "VARCHAR(64) NOT NULL DEFAULT 'transfer'",
            }],
            // Migration 5: Clear sync tables to force re-sync
            &[
                Migration::Sql("DELETE FROM sync_outgoing"),
                Migration::Sql("DELETE FROM sync_incoming"),
                Migration::Sql("DELETE FROM sync_state"),
                Migration::Sql("UPDATE sync_revision SET revision = 0"),
                Migration::Sql("DELETE FROM settings WHERE `key` = 'sync_initial_complete'"),
            ],
            // Migration 6: Add htlc_status and htlc_expiry_time to lightning payments
            &[
                Migration::AddColumn {
                    table: "payment_details_lightning",
                    column: "htlc_status",
                    definition: "VARCHAR(64) NOT NULL DEFAULT 'WaitingForPreimage'",
                },
                Migration::AddColumn {
                    table: "payment_details_lightning",
                    column: "htlc_expiry_time",
                    definition: "BIGINT NOT NULL DEFAULT 0",
                },
            ],
            // Migration 7: Backfill htlc_status for existing Lightning payments
            &[
                Migration::Sql(
                    "UPDATE payment_details_lightning
                     SET htlc_status = CASE
                             WHEN (SELECT status FROM payments WHERE id = payment_id) = 'completed' THEN 'PreimageShared'
                             WHEN (SELECT status FROM payments WHERE id = payment_id) = 'pending' THEN 'WaitingForPreimage'
                             ELSE 'Returned'
                         END",
                ),
                Migration::Sql(
                    "UPDATE settings
                     SET value = JSON_SET(value, '$.offset', 0)
                     WHERE `key` = 'sync_offset' AND value IS NOT NULL",
                ),
            ],
            // Migration 8: lnurl_receive_metadata preimage column (added then later dropped)
            &[
                Migration::AddColumn {
                    table: "lnurl_receive_metadata",
                    column: "preimage",
                    definition: "VARCHAR(255) NULL",
                },
                Migration::Sql("DELETE FROM settings WHERE `key` = 'lnurl_metadata_updated_after'"),
            ],
            // Migration 9: Clear cached lightning address (schema changed)
            &[Migration::Sql(
                "DELETE FROM settings WHERE `key` = 'lightning_address'",
            )],
            // Migration 10: Index on payment_hash for JOIN with lnurl_receive_metadata
            &[Migration::CreateIndex {
                name: "idx_payment_details_lightning_payment_hash",
                table: "payment_details_lightning",
                columns: "(payment_hash)",
            }],
            // Migration 11: Contacts table
            &[Migration::Sql(
                "CREATE TABLE IF NOT EXISTS contacts (
                        id VARCHAR(255) NOT NULL PRIMARY KEY,
                        name VARCHAR(255) NOT NULL,
                        payment_identifier VARCHAR(255) NOT NULL,
                        created_at BIGINT NOT NULL,
                        updated_at BIGINT NOT NULL
                    )",
            )],
            // Migration 12: Drop preimage column from lnurl_receive_metadata
            &[Migration::DropColumn {
                table: "lnurl_receive_metadata",
                column: "preimage",
            }],
            // Migration 13: Clear cached lightning address again (format changed)
            &[Migration::Sql(
                "DELETE FROM settings WHERE `key` = 'lightning_address'",
            )],
            // Migration 14: Add is_mature to unclaimed_deposits
            &[Migration::AddColumn {
                table: "unclaimed_deposits",
                column: "is_mature",
                definition: "TINYINT(1) NOT NULL DEFAULT 1",
            }],
            // Migration 15: Add conversion_status to payment_metadata
            &[Migration::AddColumn {
                table: "payment_metadata",
                column: "conversion_status",
                definition: "VARCHAR(64) NULL",
            }],
        ]
    }
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

#[async_trait]
impl Storage for MysqlStorage {
    #[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
    async fn list_payments(
        &self,
        request: StorageListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let mut where_clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

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
                let conversion_filter = match payment_details_filter {
                    StoragePaymentDetailsFilter::Spark {
                        conversion_refund_needed: Some(v),
                        ..
                    } => Some((v, "p.spark = 1")),
                    StoragePaymentDetailsFilter::Token {
                        conversion_refund_needed: Some(v),
                        ..
                    } => Some((v, "p.spark IS NULL")),
                    _ => None,
                };
                if let Some((conversion_refund_needed, type_check)) = conversion_filter {
                    let refund_needed = if *conversion_refund_needed {
                        "= 'RefundNeeded'"
                    } else {
                        "!= 'RefundNeeded'"
                    };
                    payment_details_clauses.push(format!(
                        "{type_check} AND pm.conversion_info IS NOT NULL AND
                         JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, '$.status')) {refund_needed}"
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

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

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

    #[allow(clippy::too_many_lines)]
    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_db_error)?;

        let (withdraw_tx_id, deposit_tx_id, spark): (Option<&str>, Option<&str>, Option<bool>) =
            match &payment.details {
                Some(PaymentDetails::Withdraw { tx_id }) => (Some(tx_id.as_str()), None, None),
                Some(PaymentDetails::Deposit { tx_id }) => (None, Some(tx_id.as_str()), None),
                Some(PaymentDetails::Spark { .. }) => (None, None, Some(true)),
                _ => (None, None, None),
            };

        tx.exec_drop(
            "INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    payment_type = VALUES(payment_type),
                    status = VALUES(status),
                    amount = VALUES(amount),
                    fees = VALUES(fees),
                    timestamp = VALUES(timestamp),
                    method = VALUES(method),
                    withdraw_tx_id = VALUES(withdraw_tx_id),
                    deposit_tx_id = VALUES(deposit_tx_id),
                    spark = VALUES(spark)",
            (
                &payment.id,
                payment.payment_type.to_string(),
                payment.status.to_string(),
                payment.amount.to_string(),
                payment.fees.to_string(),
                i64::try_from(payment.timestamp)?,
                Some(payment.method.to_string()),
                withdraw_tx_id.map(str::to_string),
                deposit_tx_id.map(str::to_string),
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
                        "INSERT INTO payment_details_spark (payment_id, invoice_details, htlc_details)
                             VALUES (?, ?, ?)
                             ON DUPLICATE KEY UPDATE
                                invoice_details = COALESCE(VALUES(invoice_details), invoice_details),
                                htlc_details = COALESCE(VALUES(htlc_details), htlc_details)",
                        (&payment.id, invoice_json, htlc_json),
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
                    "INSERT INTO payment_details_token (payment_id, metadata, tx_hash, tx_type, invoice_details)
                         VALUES (?, ?, ?, ?, ?)
                         ON DUPLICATE KEY UPDATE
                            metadata = VALUES(metadata),
                            tx_hash = VALUES(tx_hash),
                            tx_type = VALUES(tx_type),
                            invoice_details = COALESCE(VALUES(invoice_details), invoice_details)",
                    (
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
                    "INSERT INTO payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, description, preimage, htlc_status, htlc_expiry_time)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                         ON DUPLICATE KEY UPDATE
                            invoice = VALUES(invoice),
                            payment_hash = VALUES(payment_hash),
                            destination_pubkey = VALUES(destination_pubkey),
                            description = VALUES(description),
                            preimage = COALESCE(VALUES(preimage), preimage),
                            htlc_status = COALESCE(VALUES(htlc_status), htlc_status),
                            htlc_expiry_time = COALESCE(VALUES(htlc_expiry_time), htlc_expiry_time)",
                    (
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
            Some(PaymentDetails::Withdraw { .. } | PaymentDetails::Deposit { .. }) | None => {}
        }

        tx.commit().await.map_err(map_db_error)?;
        Ok(())
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
            "INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info, conversion_status)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
                parent_payment_id = COALESCE(VALUES(parent_payment_id), parent_payment_id),
                lnurl_pay_info = COALESCE(VALUES(lnurl_pay_info), lnurl_pay_info),
                lnurl_withdraw_info = COALESCE(VALUES(lnurl_withdraw_info), lnurl_withdraw_info),
                lnurl_description = COALESCE(VALUES(lnurl_description), lnurl_description),
                conversion_info = COALESCE(VALUES(conversion_info), conversion_info),
                conversion_status = COALESCE(VALUES(conversion_status), conversion_status)",
            (
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
            "INSERT INTO settings (`key`, value) VALUES (?, ?)
             ON DUPLICATE KEY UPDATE value = VALUES(value)",
            (key, value),
        )
        .await
        .map_err(map_db_error)?;

        Ok(())
    }

    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        let row: Option<String> = conn
            .exec_first("SELECT value FROM settings WHERE `key` = ?", (key,))
            .await
            .map_err(map_db_error)?;

        Ok(row)
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;

        conn.exec_drop("DELETE FROM settings WHERE `key` = ?", (key,))
            .await
            .map_err(map_db_error)?;

        Ok(())
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.id = ?");
        let row: Option<Row> = conn.exec_first(&query, (id,)).await.map_err(map_db_error)?;
        let row = row.ok_or(StorageError::NotFound)?;
        map_payment(&row)
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE l.invoice = ?");
        let row: Option<Row> = conn
            .exec_first(&query, (invoice,))
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
            .query_first::<i64, _>(
                "SELECT EXISTS(SELECT 1 FROM payment_metadata WHERE parent_payment_id IS NOT NULL LIMIT 1)",
            )
            .await
            .map_err(map_db_error)?
            .is_some_and(|v| v != 0);

        if !has_related {
            return Ok(HashMap::new());
        }

        let placeholders = build_placeholders(parent_payment_ids.len());
        let query = format!(
            "{SELECT_PAYMENT_SQL} WHERE pm.parent_payment_id IN ({placeholders}) ORDER BY p.timestamp ASC"
        );

        let params: Vec<Value> = parent_payment_ids
            .iter()
            .cloned()
            .map(Value::from)
            .collect();

        let rows: Vec<Row> = conn
            .exec(&query, Params::Positional(params))
            .await
            .map_err(map_db_error)?;

        let mut result: HashMap<String, Vec<Payment>> = HashMap::new();
        for row in &rows {
            let payment = map_payment(row)?;
            let parent_payment_id: String = row
                .get(31)
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
            "INSERT INTO unclaimed_deposits (txid, vout, amount_sats, is_mature)
             VALUES (?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE is_mature = VALUES(is_mature), amount_sats = VALUES(amount_sats)",
            (
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
            "DELETE FROM unclaimed_deposits WHERE txid = ? AND vout = ?",
            (txid, i32::try_from(vout)?),
        )
        .await
        .map_err(map_db_error)?;
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let mut conn = self.pool.get_conn().await.map_err(map_db_error)?;
        let rows: Vec<Row> = conn
            .query(
                "SELECT txid, vout, amount_sats, is_mature, claim_error, refund_tx, refund_tx_id FROM unclaimed_deposits",
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
                    "UPDATE unclaimed_deposits SET claim_error = ?, refund_tx = NULL, refund_tx_id = NULL WHERE txid = ? AND vout = ?",
                    (error_json, txid, i32::try_from(vout)?),
                )
                .await
                .map_err(map_db_error)?;
            }
            UpdateDepositPayload::Refund {
                refund_txid,
                refund_tx,
            } => {
                conn.exec_drop(
                    "UPDATE unclaimed_deposits SET refund_tx = ?, refund_tx_id = ?, claim_error = NULL WHERE txid = ? AND vout = ?",
                    (refund_tx, refund_txid, txid, i32::try_from(vout)?),
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
                "INSERT INTO lnurl_receive_metadata (payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
                 VALUES (?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    nostr_zap_request = VALUES(nostr_zap_request),
                    nostr_zap_receipt = VALUES(nostr_zap_receipt),
                    sender_comment = VALUES(sender_comment)",
                (m.payment_hash, m.nostr_zap_request, m.nostr_zap_receipt, m.sender_comment),
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
                 FROM contacts ORDER BY name ASC LIMIT ? OFFSET ?",
                (limit, offset),
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
                 FROM contacts WHERE id = ?",
                (id,),
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
            "INSERT INTO contacts (id, name, payment_identifier, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
               name = VALUES(name),
               payment_identifier = VALUES(payment_identifier),
               updated_at = VALUES(updated_at)",
            (
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
        conn.exec_drop("DELETE FROM contacts WHERE id = ?", (id,))
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
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_db_error)?;

        let local_revision: i64 = tx
            .query_first("SELECT COALESCE(MAX(revision), 0) + 1 FROM sync_outgoing")
            .await
            .map_err(map_db_error)?
            .unwrap_or(1);

        let updated_fields_json = serde_json::to_string(&record.updated_fields)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.exec_drop(
            "INSERT INTO sync_outgoing (record_type, data_id, schema_version, commit_time, updated_fields_json, revision)
                 VALUES (?, ?, ?, ?, ?, ?)",
            (
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
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_db_error)?;

        let mut result = tx
            .exec_iter(
                "DELETE FROM sync_outgoing WHERE record_type = ? AND data_id = ? AND revision = ?",
                (
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
                "complete_outgoing_sync: DELETE from sync_outgoing matched 0 rows \
                 (type={}, data_id={}, revision={})",
                record.id.r#type, record.id.data_id, local_revision
            );
        }

        let data_json = serde_json::to_string(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.exec_drop(
            "INSERT INTO sync_state (record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    schema_version = VALUES(schema_version),
                    commit_time = VALUES(commit_time),
                    data = VALUES(data),
                    revision = VALUES(revision)",
            (
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
            "UPDATE sync_revision SET revision = GREATEST(revision, ?)",
            (i64::try_from(record.revision)?,),
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
                 FROM sync_outgoing o
                 LEFT JOIN sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id
                 ORDER BY o.revision ASC
                 LIMIT ?",
                (i64::from(limit),),
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

        let revision: i64 = conn
            .query_first("SELECT revision FROM sync_revision")
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
                "INSERT INTO sync_incoming (record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    schema_version = VALUES(schema_version),
                    commit_time = VALUES(commit_time),
                    data = VALUES(data)",
                (
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
            "DELETE FROM sync_incoming WHERE record_type = ? AND data_id = ? AND revision = ?",
            (
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
                 FROM sync_incoming i
                 LEFT JOIN sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id
                 ORDER BY i.revision ASC
                 LIMIT ?",
                (i64::from(limit),),
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
            .query_first(
                "SELECT o.record_type, o.data_id, o.schema_version, o.commit_time, o.updated_fields_json, o.revision,
                        e.schema_version AS existing_schema_version, e.commit_time AS existing_commit_time, e.data AS existing_data, e.revision AS existing_revision
                 FROM sync_outgoing o
                 LEFT JOIN sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id
                 ORDER BY o.revision DESC
                 LIMIT 1",
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
            .start_transaction(TxOpts::default())
            .await
            .map_err(map_db_error)?;

        let data_json = serde_json::to_string(&record.data)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let commit_time = chrono::Utc::now().timestamp();

        tx.exec_drop(
            "INSERT INTO sync_state (record_type, data_id, schema_version, commit_time, data, revision)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON DUPLICATE KEY UPDATE
                    schema_version = VALUES(schema_version),
                    commit_time = VALUES(commit_time),
                    data = VALUES(data),
                    revision = VALUES(revision)",
            (
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
            "UPDATE sync_revision SET revision = GREATEST(revision, ?)",
            (i64::try_from(record.revision)?,),
        )
        .await
        .map_err(map_db_error)?;

        tx.commit().await.map_err(map_db_error)?;

        Ok(())
    }
}

/// Base query for payment lookups. Indices 0-30 are used by `map_payment`,
/// index 31 (`parent_payment_id`) is only used by `get_payments_by_parent_ids`.
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
      FROM payments p
      LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
      LEFT JOIN payment_details_token t ON p.id = t.payment_id
      LEFT JOIN payment_details_spark s ON p.id = s.payment_id
      LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
      LEFT JOIN lnurl_receive_metadata lrm ON l.payment_hash = lrm.payment_hash";

#[allow(clippy::too_many_lines)]
fn map_payment(row: &Row) -> Result<Payment, StorageError> {
    let withdraw_tx_id: Option<String> = get_opt_str(row, 7);
    let deposit_tx_id: Option<String> = get_opt_str(row, 8);
    let spark: Option<bool> = get_opt_bool(row, 9);
    let lightning_invoice: Option<String> = get_opt_str(row, 10);
    let token_metadata: Option<String> = get_opt_str(row, 20);

    let details = match (
        lightning_invoice,
        withdraw_tx_id,
        deposit_tx_id,
        spark,
        token_metadata,
    ) {
        (Some(invoice), _, _, _, _) => {
            let payment_hash: String = get_str(row, 11)?;
            let destination_pubkey: String = get_str(row, 12)?;
            let description: Option<String> = get_opt_str(row, 13);
            let preimage: Option<String> = get_opt_str(row, 14);
            let htlc_status_str: Option<String> = get_opt_str(row, 15);
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
            let htlc_expiry_time: i64 = get_i64(row, 16)?;
            let htlc_details = SparkHtlcDetails {
                payment_hash,
                preimage,
                expiry_time: u64::try_from(htlc_expiry_time)?,
                status: htlc_status,
            };
            let lnurl_pay_info_str: Option<String> = get_opt_str(row, 17);
            let lnurl_withdraw_info_str: Option<String> = get_opt_str(row, 18);
            let lnurl_nostr_zap_request: Option<String> = get_opt_str(row, 26);
            let lnurl_nostr_zap_receipt: Option<String> = get_opt_str(row, 27);
            let lnurl_sender_comment: Option<String> = get_opt_str(row, 28);
            let lnurl_payment_hash: Option<String> = get_opt_str(row, 29);

            let lnurl_pay_info: Option<LnurlPayInfo> = from_json_string_opt(lnurl_pay_info_str)?;
            let lnurl_withdraw_info: Option<LnurlWithdrawInfo> =
                from_json_string_opt(lnurl_withdraw_info_str)?;

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
            })
        }
        (_, Some(tx_id), _, _, _) => Some(PaymentDetails::Withdraw { tx_id }),
        (_, _, Some(tx_id), _, _) => Some(PaymentDetails::Deposit { tx_id }),
        (_, _, _, Some(_), _) => {
            let invoice_details_str: Option<String> = get_opt_str(row, 24);
            let invoice_details = from_json_string_opt(invoice_details_str)?;
            let htlc_details_str: Option<String> = get_opt_str(row, 25);
            let htlc_details = from_json_string_opt(htlc_details_str)?;
            let conversion_info_str: Option<String> = get_opt_str(row, 19);
            let conversion_info: Option<ConversionInfo> =
                from_json_string_opt(conversion_info_str)?;
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                conversion_info,
            })
        }
        (_, _, _, _, Some(metadata_str)) => {
            let tx_type_str: String = get_str(row, 22)?;
            let tx_type = tx_type_str
                .parse()
                .map_err(|e: String| StorageError::Serialization(e))?;
            let invoice_details_str: Option<String> = get_opt_str(row, 23);
            let invoice_details = from_json_string_opt(invoice_details_str)?;
            let conversion_info_str: Option<String> = get_opt_str(row, 19);
            let conversion_info: Option<ConversionInfo> =
                from_json_string_opt(conversion_info_str)?;
            Some(PaymentDetails::Token {
                metadata: serde_json::from_str(&metadata_str)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                tx_hash: get_str(row, 21)?,
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
            let conversion_status_str: Option<String> = get_opt_str(row, 30);
            conversion_status_str
                .map(|s| {
                    s.parse::<ConversionStatus>()
                        .map(|status| ConversionDetails {
                            status,
                            from: None,
                            to: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers::{ContainerAsync, runners::AsyncRunner};
    use testcontainers_modules::mysql::Mysql;

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

            let storage = MysqlStorage::new(MysqlStorageConfig::with_defaults(connection_string))
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
    async fn test_conversion_refund_needed_filtering() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_conversion_refund_needed_filtering(Box::new(fixture.storage))
            .await;
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
    async fn test_payment_metadata_merge() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_payment_metadata_merge(Box::new(fixture.storage)).await;
    }

    #[tokio::test]
    async fn test_conversion_status_persistence() {
        let fixture = MysqlTestFixture::new().await;
        crate::persist::tests::test_conversion_status_persistence(Box::new(fixture.storage)).await;
    }
}
