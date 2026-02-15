use std::path::{Path, PathBuf};

use macros::async_trait;
use rusqlite::{
    Connection, Row, ToSql, params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
};
use rusqlite_migration::{M, Migrations, SchemaVersion};

use crate::{
    AssetFilter, ConversionInfo, DepositInfo, ListPaymentsRequest, LnurlPayInfo,
    LnurlReceiveMetadata, LnurlWithdrawInfo, PaymentDetails, PaymentDetailsFilter, PaymentMethod,
    TokenTransactionType,
    error::DepositClaimError,
    persist::{PaymentMetadata, SetLnurlMetadataItem, UpdateDepositPayload},
    sync_storage::{
        IncomingChange, OutgoingChange, Record, RecordChange, RecordId, UnversionedRecordChange,
    },
};

use std::collections::HashMap;

use tracing::warn;

use super::{Payment, Storage, StorageError};

const DEFAULT_DB_FILENAME: &str = "storage.sql";
/// SQLite-based storage implementation
pub struct SqliteStorage {
    db_dir: PathBuf,
}

impl SqliteStorage {
    /// Creates a new `SQLite` storage
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the `SQLite` database file
    ///
    /// # Returns
    ///
    /// A new `SqliteStorage` instance or an error
    pub fn new(path: &Path) -> Result<Self, StorageError> {
        let storage = Self {
            db_dir: path.to_path_buf(),
        };

        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        std::fs::create_dir_all(path)
            .map_err(|e| StorageError::InitializationError(e.to_string()))?;

        storage.migrate()?;
        Ok(storage)
    }

    pub(crate) fn get_connection(&self) -> Result<Connection, StorageError> {
        Ok(Connection::open(self.get_db_path())?)
    }

    fn get_db_path(&self) -> PathBuf {
        self.db_dir.join(DEFAULT_DB_FILENAME)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let migrations =
            Migrations::new(Self::current_migrations().into_iter().map(M::up).collect());
        let mut conn = self.get_connection()?;
        let previous_version = match migrations.current_version(&conn)? {
            SchemaVersion::Inside(previous_version) => previous_version.get(),
            _ => 0,
        };
        migrations.to_latest(&mut conn)?;

        if previous_version < 6 {
            Self::migrate_lnurl_metadata_description(&mut conn)?;
        }

        Ok(())
    }

    fn migrate_lnurl_metadata_description(conn: &mut Connection) -> Result<(), StorageError> {
        let mut stmt = conn.prepare("SELECT payment_id, lnurl_pay_info FROM payment_metadata")?;
        let pay_infos: Vec<_> = stmt
            .query_map([], |row| {
                let payment_id: String = row.get(0)?;
                let lnurl_pay_info: Option<LnurlPayInfo> = row.get(1)?;
                Ok((payment_id, lnurl_pay_info))
            })?
            .collect::<Result<_, _>>()?;
        let pay_infos = pay_infos
            .into_iter()
            .filter_map(|(payment_id, lnurl_pay_info)| {
                let pay_info = lnurl_pay_info?;
                let description = pay_info.extract_description()?;
                Some((payment_id, description))
            })
            .collect::<Vec<_>>();

        for pay_info in pay_infos {
            conn.execute(
                "UPDATE payment_metadata SET lnurl_description = ? WHERE payment_id = ?",
                params![pay_info.1, pay_info.0],
            )?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn current_migrations() -> Vec<&'static str> {
        vec![
            "CREATE TABLE IF NOT EXISTS payments (
              id TEXT PRIMARY KEY,
              payment_type TEXT NOT NULL,
              status TEXT NOT NULL,
              amount INTEGER NOT NULL,
              fees INTEGER NOT NULL,
              timestamp INTEGER NOT NULL,
              details TEXT,
              method TEXT
            );",
            "CREATE TABLE IF NOT EXISTS settings (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );",
            "CREATE TABLE IF NOT EXISTS unclaimed_deposits (
              txid TEXT NOT NULL,
              vout INTEGER NOT NULL,
              amount_sats INTEGER,
              claim_error TEXT,
              refund_tx TEXT,
              refund_tx_id TEXT,
              PRIMARY KEY (txid, vout)
            );",
            "CREATE TABLE IF NOT EXISTS payment_metadata (
              payment_id TEXT PRIMARY KEY,
              lnurl_pay_info TEXT
            );",
            "CREATE TABLE IF NOT EXISTS deposit_refunds (
              deposit_tx_id TEXT NOT NULL,
              deposit_vout INTEGER NOT NULL,
              refund_tx TEXT NOT NULL,
              refund_tx_id TEXT NOT NULL,
              PRIMARY KEY (deposit_tx_id, deposit_vout)              
            );",
            "ALTER TABLE payment_metadata ADD COLUMN lnurl_description TEXT;",
            "
            ALTER TABLE payments ADD COLUMN withdraw_tx_id TEXT;
            ALTER TABLE payments ADD COLUMN deposit_tx_id TEXT;
            ALTER TABLE payments ADD COLUMN spark INTEGER;
            CREATE TABLE payment_details_lightning (
              payment_id TEXT PRIMARY KEY,
              invoice TEXT NOT NULL,
              payment_hash TEXT NOT NULL,
              destination_pubkey TEXT NOT NULL,
              description TEXT,
              preimage TEXT,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            );
            INSERT INTO payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, description, preimage)
            SELECT id, json_extract(details, '$.Lightning.invoice'), json_extract(details, '$.Lightning.payment_hash'), 
                json_extract(details, '$.Lightning.destination_pubkey'), json_extract(details, '$.Lightning.description'), 
                json_extract(details, '$.Lightning.preimage') 
            FROM payments WHERE json_extract(details, '$.Lightning.invoice') IS NOT NULL;

            UPDATE payments SET withdraw_tx_id = json_extract(details, '$.Withdraw.tx_id')
            WHERE json_extract(details, '$.Withdraw.tx_id') IS NOT NULL;

            UPDATE payments SET deposit_tx_id = json_extract(details, '$.Deposit.tx_id')
            WHERE json_extract(details, '$.Deposit.tx_id') IS NOT NULL;

            ALTER TABLE payments DROP COLUMN details;

            CREATE INDEX idx_payment_details_lightning_invoice ON payment_details_lightning(invoice);
            ",
            "CREATE TABLE payment_details_token (
              payment_id TEXT PRIMARY KEY,
              metadata TEXT NOT NULL,
              tx_hash TEXT NOT NULL,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            );",
            // Migration to change payments amount and fees from INTEGER to TEXT
            "CREATE TABLE payments_new (
              id TEXT PRIMARY KEY,
              payment_type TEXT NOT NULL,
              status TEXT NOT NULL,
              amount TEXT NOT NULL,
              fees TEXT NOT NULL,
              timestamp INTEGER NOT NULL,
              method TEXT,
              withdraw_tx_id TEXT,
              deposit_tx_id TEXT,
              spark INTEGER
            );",
            "INSERT INTO payments_new (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
             SELECT id, payment_type, status, CAST(amount AS TEXT), CAST(fees AS TEXT), timestamp, method, withdraw_tx_id, deposit_tx_id, spark
             FROM payments;",
            "DROP TABLE payments;",
            "ALTER TABLE payments_new RENAME TO payments;",
            "CREATE TABLE payment_details_spark (
              payment_id TEXT NOT NULL PRIMARY KEY,
              invoice_details TEXT NOT NULL,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            );
            ALTER TABLE payment_details_token ADD COLUMN invoice_details TEXT;",
            "ALTER TABLE payment_metadata ADD COLUMN lnurl_withdraw_info TEXT;",
            // sync_revision: tracks the last committed revision (from server-acknowledged
            // or server-received records). Does NOT include pending outgoing queue ids.
            // sync_outgoing.revision stores a local queue id for ordering/de-duplication only.
            "CREATE TABLE sync_revision (
                revision INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO sync_revision (revision) VALUES (0);
            CREATE TABLE sync_outgoing(
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time INTEGER NOT NULL,
                updated_fields_json TEXT NOT NULL,
                revision INTEGER NOT NULL
            );
            CREATE INDEX idx_sync_outgoing_data_id_record_type ON sync_outgoing(record_type, data_id);
            CREATE TABLE sync_state(
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time INTEGER NOT NULL,
                data TEXT NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY(record_type, data_id)
            );",
            "CREATE TABLE sync_incoming(
                record_type TEXT NOT NULL,
                data_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                commit_time INTEGER NOT NULL,
                data TEXT NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY(record_type, data_id, revision)
            );
            CREATE INDEX idx_sync_incoming_revision ON sync_incoming(revision);",
            "ALTER TABLE payment_details_spark RENAME TO tmp_payment_details_spark;
            CREATE TABLE payment_details_spark (
              payment_id TEXT NOT NULL PRIMARY KEY,
              invoice_details TEXT,
              htlc_details TEXT,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            );
            INSERT INTO payment_details_spark (payment_id, invoice_details)
             SELECT payment_id, invoice_details FROM tmp_payment_details_spark;
            DROP TABLE tmp_payment_details_spark;",
            "CREATE TABLE lnurl_receive_metadata (
                payment_hash TEXT NOT NULL PRIMARY KEY,
                nostr_zap_request TEXT,
                nostr_zap_receipt TEXT,
                sender_comment TEXT
            );",
            // Delete all unclaimed deposits to clear old claim_error JSON format.
            // Deposits will be recovered on next sync.
            "DELETE FROM unclaimed_deposits;",
            // Clear all sync tables due to BreezSigner signature change.
            // This forces users to sync from scratch to the sync server.
            // Also delete the sync_initial_complete flag to force re-populating
            // all payment metadata for outgoing sync using the new key.
            "DELETE FROM sync_outgoing;
             DELETE FROM sync_incoming;
             DELETE FROM sync_state;
             UPDATE sync_revision SET revision = 0;
             DELETE FROM settings WHERE key = 'sync_initial_complete';",
            "ALTER TABLE payment_metadata ADD COLUMN token_conversion_info TEXT;",
            "ALTER TABLE payment_metadata ADD COLUMN parent_payment_id TEXT;",
            "
            ALTER TABLE payment_metadata DROP COLUMN token_conversion_info;
            ALTER TABLE payment_metadata ADD COLUMN conversion_info TEXT;
            ",
            // Add tx_type column with a default value of 'transfer'.
            // Reset only the token sync position (not bitcoin offset) to trigger token re-sync.
            // This will update all token payment records with the correct tx_type values.
            // Note: This intentionally couples to the CachedSyncInfo schema at migration time.
            "ALTER TABLE payment_details_token ADD COLUMN tx_type TEXT NOT NULL DEFAULT 'transfer';
            UPDATE settings
            SET value = json_set(value, '$.last_synced_final_token_payment_id', NULL)
            WHERE key = 'sync_offset' AND json_valid(value) AND json_type(value, '$.last_synced_final_token_payment_id') IS NOT NULL;",
            "DELETE FROM sync_outgoing;
             DELETE FROM sync_incoming;
             DELETE FROM sync_state;
             UPDATE sync_revision SET revision = 0;
             DELETE FROM settings WHERE key = 'sync_initial_complete';",
        ]
    }
}

/// Maps a rusqlite error to the appropriate `StorageError`.
/// Database busy/locked errors are mapped to `Connection` (transient),
/// other errors are mapped to `Implementation`.
#[allow(clippy::needless_pass_by_value)]
fn map_sqlite_error(e: rusqlite::Error) -> StorageError {
    match e {
        rusqlite::Error::SqliteFailure(err, _)
            if err.code == rusqlite::ErrorCode::DatabaseBusy
                || err.code == rusqlite::ErrorCode::DatabaseLocked =>
        {
            StorageError::Connection(e.to_string())
        }
        _ => StorageError::Implementation(e.to_string()),
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        map_sqlite_error(value)
    }
}

impl From<rusqlite_migration::Error> for StorageError {
    fn from(value: rusqlite_migration::Error) -> Self {
        StorageError::Implementation(value.to_string())
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    #[allow(clippy::too_many_lines)]
    async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError> {
        let connection = self.get_connection()?;

        // Build WHERE clauses based on filters
        let mut where_clauses = Vec::new();
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();

        // Filter by payment type
        if let Some(ref type_filter) = request.type_filter
            && !type_filter.is_empty()
        {
            let placeholders = type_filter
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", ");
            where_clauses.push(format!("p.payment_type IN ({placeholders})"));
            for payment_type in type_filter {
                params.push(Box::new(payment_type.to_string()));
            }
        }

        // Filter by status
        if let Some(ref status_filter) = request.status_filter
            && !status_filter.is_empty()
        {
            let placeholders = status_filter
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", ");
            where_clauses.push(format!("p.status IN ({placeholders})"));
            for status in status_filter {
                params.push(Box::new(status.to_string()));
            }
        }

        // Filter by timestamp range
        if let Some(from_timestamp) = request.from_timestamp {
            where_clauses.push("p.timestamp >= ?".to_string());
            params.push(Box::new(from_timestamp));
        }

        if let Some(to_timestamp) = request.to_timestamp {
            where_clauses.push("p.timestamp < ?".to_string());
            params.push(Box::new(to_timestamp));
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
                        // Filter by specific token identifier
                        where_clauses
                            .push("json_extract(t.metadata, '$.identifier') = ?".to_string());
                        params.push(Box::new(identifier.clone()));
                    }
                }
            }
        }

        // Filter by payment details. If any filter matches, we include the payment
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
                    let placeholders = htlc_statuses
                        .iter()
                        .map(|_| "?")
                        .collect::<Vec<_>>()
                        .join(", ");
                    payment_details_clauses.push(format!(
                        "json_extract(s.htlc_details, '$.status') IN ({placeholders})"
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
                        PaymentDetailsFilter::Spark { .. } => "p.spark = 1",
                        PaymentDetailsFilter::Token { .. } => "p.spark IS NULL",
                    };
                    let refund_needed = if *conversion_refund_needed {
                        "= 'RefundNeeded'"
                    } else {
                        "!= 'RefundNeeded'"
                    };
                    payment_details_clauses.push(format!(
                        "{type_check} AND pm.conversion_info IS NOT NULL AND
                         json_extract(pm.conversion_info, '$.status') {refund_needed}"
                    ));
                }
                // Filter by token transaction hash
                if let PaymentDetailsFilter::Token {
                    tx_hash: Some(tx_hash),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push("t.tx_hash = ?".to_string());
                    params.push(Box::new(tx_hash.clone()));
                }

                // Filter by token transaction type
                if let PaymentDetailsFilter::Token {
                    tx_type: Some(tx_type),
                    ..
                } = payment_details_filter
                {
                    payment_details_clauses.push("t.tx_type = ?".to_string());
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

        // Exclude child payments (those with a parent_payment_id)
        // Child payments are accessed via the parent's related_payments field
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

        let query = format!(
            "{SELECT_PAYMENT_SQL} {where_sql} ORDER BY p.timestamp {order_direction} LIMIT {} OFFSET {}",
            request.limit.unwrap_or(u32::MAX),
            request.offset.unwrap_or(0)
        );

        let mut stmt = connection.prepare(&query)?;
        let param_refs: Vec<&dyn ToSql> = params.iter().map(std::convert::AsRef::as_ref).collect();
        let payments = stmt
            .query_map(param_refs.as_slice(), map_payment)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(payments)
    }

    #[allow(clippy::too_many_lines)]
    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let mut connection = self.get_connection()?;
        let tx = connection.transaction()?;
        tx.execute(
            "INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method) 
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET 
                payment_type=excluded.payment_type,
                status=excluded.status,
                amount=excluded.amount,
                fees=excluded.fees,
                timestamp=excluded.timestamp,
                method=excluded.method",
            params![
                payment.id,
                payment.payment_type.to_string(),
                payment.status.to_string(),
                U128SqlWrapper(payment.amount),
                U128SqlWrapper(payment.fees),
                payment.timestamp,
                payment.method,
            ],
        )?;

        match payment.details {
            Some(PaymentDetails::Withdraw { tx_id }) => {
                tx.execute(
                    "UPDATE payments SET withdraw_tx_id = ? WHERE id = ?",
                    params![tx_id, payment.id],
                )?;
            }
            Some(PaymentDetails::Deposit { tx_id }) => {
                tx.execute(
                    "UPDATE payments SET deposit_tx_id = ? WHERE id = ?",
                    params![tx_id, payment.id],
                )?;
            }
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                ..
            }) => {
                tx.execute(
                    "UPDATE payments SET spark = 1 WHERE id = ?",
                    params![payment.id],
                )?;
                if invoice_details.is_some() || htlc_details.is_some() {
                    // Upsert both details together and avoid overwriting existing data with NULLs
                    tx.execute(
                        "INSERT INTO payment_details_spark (payment_id, invoice_details, htlc_details)
                         VALUES (?, ?, ?)
                         ON CONFLICT(payment_id) DO UPDATE SET
                            invoice_details=COALESCE(excluded.invoice_details, payment_details_spark.invoice_details),
                            htlc_details=COALESCE(excluded.htlc_details, payment_details_spark.htlc_details)",
                        params![
                            payment.id,
                            invoice_details.as_ref().map(serde_json::to_string).transpose()?,
                            htlc_details.as_ref().map(serde_json::to_string).transpose()?,
                        ],
                    )?;
                }
            }
            Some(PaymentDetails::Token {
                metadata,
                tx_hash,
                tx_type,
                invoice_details,
                ..
            }) => {
                tx.execute(
                    "INSERT INTO payment_details_token (payment_id, metadata, tx_hash, tx_type, invoice_details)
                     VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT(payment_id) DO UPDATE SET 
                        metadata=excluded.metadata,
                        tx_hash=excluded.tx_hash,
                        tx_type=excluded.tx_type,
                        invoice_details=COALESCE(excluded.invoice_details, payment_details_token.invoice_details)",
                    params![
                        payment.id,
                        serde_json::to_string(&metadata)?,
                        tx_hash,
                        tx_type.to_string(),
                        invoice_details.as_ref().map(serde_json::to_string).transpose()?,
                    ],
                )?;
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
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT(payment_id) DO UPDATE SET
                        invoice=excluded.invoice,
                        payment_hash=excluded.payment_hash,
                        destination_pubkey=excluded.destination_pubkey,
                        description=excluded.description,
                        preimage=COALESCE(excluded.preimage, payment_details_lightning.preimage)",
                    params![
                        payment.id,
                        invoice,
                        payment_hash,
                        destination_pubkey,
                        description,
                        preimage,
                    ],
                )?;
            }
            None => {}
        }

        tx.commit()?;
        Ok(())
    }

    async fn insert_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(payment_id) DO UPDATE SET
                parent_payment_id = COALESCE(excluded.parent_payment_id, parent_payment_id),
                lnurl_pay_info = COALESCE(excluded.lnurl_pay_info, lnurl_pay_info),
                lnurl_withdraw_info = COALESCE(excluded.lnurl_withdraw_info, lnurl_withdraw_info),
                lnurl_description = COALESCE(excluded.lnurl_description, lnurl_description),
                conversion_info = COALESCE(excluded.conversion_info, conversion_info)",
            params![
                payment_id,
                metadata.parent_payment_id,
                metadata.lnurl_pay_info,
                metadata.lnurl_withdraw_info,
                metadata.lnurl_description,
                metadata.conversion_info.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;

        Ok(())
    }

    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
            params![key, value],
        )?;

        Ok(())
    }

    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection.prepare("SELECT value FROM settings WHERE key = ?")?;

        let result = stmt.query_row(params![key], |row| {
            let value_str: String = row.get(0)?;
            Ok(value_str)
        });

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute("DELETE FROM settings WHERE key = ?", params![key])?;

        Ok(())
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let connection = self.get_connection()?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE p.id = ?");
        let mut stmt = connection.prepare(&query)?;
        let payment = stmt.query_row(params![id], map_payment)?;
        Ok(payment)
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        let connection = self.get_connection()?;
        let query = format!("{SELECT_PAYMENT_SQL} WHERE l.invoice = ?");
        let mut stmt = connection.prepare(&query)?;
        let payment = stmt.query_row(params![invoice], map_payment);
        match payment {
            Ok(payment) => Ok(Some(payment)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn get_payments_by_parent_ids(
        &self,
        parent_payment_ids: Vec<String>,
    ) -> Result<HashMap<String, Vec<Payment>>, StorageError> {
        if parent_payment_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let connection = self.get_connection()?;

        // Early exit if no related payments exist
        let has_related: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM payment_metadata WHERE parent_payment_id IS NOT NULL LIMIT 1)",
            [],
            |row| row.get(0),
        )?;
        if !has_related {
            return Ok(HashMap::new());
        }

        // Build the IN clause with placeholders
        let placeholders: Vec<&str> = parent_payment_ids.iter().map(|_| "?").collect();
        let in_clause = placeholders.join(", ");

        let query = format!(
            "{SELECT_PAYMENT_SQL} WHERE pm.parent_payment_id IN ({in_clause}) ORDER BY p.timestamp ASC"
        );

        let mut stmt = connection.prepare(&query)?;
        let params: Vec<&dyn ToSql> = parent_payment_ids
            .iter()
            .map(|id| id as &dyn ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), |row| {
            let payment = map_payment(row)?;
            let parent_payment_id: String = row.get(27)?;
            Ok((parent_payment_id, payment))
        })?;

        let mut result: HashMap<String, Vec<Payment>> = HashMap::new();
        for row in rows {
            let (parent_id, related_payment) = row?;
            result.entry(parent_id).or_default().push(related_payment);
        }

        Ok(result)
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO unclaimed_deposits (txid, vout, amount_sats) 
             VALUES (?, ?, ?)",
            params![txid, vout, amount_sats,],
        )?;
        Ok(())
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        connection.execute(
            "DELETE FROM unclaimed_deposits WHERE txid = ? AND vout = ?",
            params![txid, vout],
        )?;
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let connection = self.get_connection()?;
        let mut stmt =
            connection.prepare("SELECT txid, vout, amount_sats, claim_error, refund_tx, refund_tx_id FROM unclaimed_deposits")?;
        let rows = stmt.query_map(params![], |row| {
            Ok(DepositInfo {
                txid: row.get(0)?,
                vout: row.get(1)?,
                amount_sats: row.get(2)?,
                claim_error: row.get(3)?,
                refund_tx: row.get(4)?,
                refund_tx_id: row.get(5)?,
            })
        })?;
        let mut deposits = Vec::new();
        for row in rows {
            deposits.push(row?);
        }
        Ok(deposits)
    }

    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        match payload {
            UpdateDepositPayload::ClaimError { error } => {
                connection.execute(
                    "UPDATE unclaimed_deposits SET claim_error = ? WHERE txid = ? AND vout = ?",
                    params![error, txid, vout],
                )?;
            }
            UpdateDepositPayload::Refund {
                refund_txid,
                refund_tx,
            } => {
                connection.execute(
                    "UPDATE unclaimed_deposits SET refund_tx = ?, refund_tx_id = ? WHERE txid = ? AND vout = ?",
                    params![refund_tx, refund_txid, txid, vout],
                )?;
            }
        }
        Ok(())
    }

    async fn set_lnurl_metadata(
        &self,
        metadata: Vec<SetLnurlMetadataItem>,
    ) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        for metadata in metadata {
            connection.execute(
                "INSERT OR REPLACE INTO lnurl_receive_metadata (payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
                 VALUES (?, ?, ?, ?)",
                params![
                    metadata.payment_hash,
                    metadata.nostr_zap_request,
                    metadata.nostr_zap_receipt,
                    metadata.sender_comment,
                ],
            )?;
        }
        Ok(())
    }

    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, StorageError> {
        let mut connection = self.get_connection()?;
        let tx = connection.transaction().map_err(map_sqlite_error)?;

        // This revision is a local queue id for pending rows, not a server revision.
        let local_revision: u64 = tx
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) + 1 FROM sync_outgoing",
                [],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)?;

        tx.execute(
            "INSERT INTO sync_outgoing (
                record_type
            ,   data_id
            ,   schema_version
            ,   commit_time
            ,   updated_fields_json
            ,   revision
            )
             VALUES (?, ?, ?, strftime('%s','now'), ?, ?)",
            params![
                record.id.r#type,
                record.id.data_id,
                record.schema_version.clone(),
                serde_json::to_string(&record.updated_fields)?,
                local_revision,
            ],
        )
        .map_err(map_sqlite_error)?;

        tx.commit().map_err(map_sqlite_error)?;
        Ok(local_revision)
    }

    async fn complete_outgoing_sync(
        &self,
        record: Record,
        local_revision: u64,
    ) -> Result<(), StorageError> {
        let mut connection = self.get_connection()?;
        let tx = connection.transaction().map_err(map_sqlite_error)?;

        let rows_deleted = tx
            .execute(
                "DELETE FROM sync_outgoing WHERE record_type = ? AND data_id = ? AND revision = ?",
                params![record.id.r#type, record.id.data_id, local_revision],
            )
            .map_err(map_sqlite_error)?;

        if rows_deleted == 0 {
            warn!(
                "complete_outgoing_sync: DELETE from sync_outgoing matched 0 rows \
                 (type={}, data_id={}, revision={})",
                record.id.r#type, record.id.data_id, local_revision
            );
        }

        tx.execute(
            "INSERT OR REPLACE INTO sync_state (
                record_type
            ,   data_id
            ,   schema_version
            ,   commit_time
            ,   data
            ,   revision
            )
             VALUES (?, ?, ?, strftime('%s','now'), ?, ?)",
            params![
                record.id.r#type,
                record.id.data_id,
                record.schema_version.clone(),
                serde_json::to_string(&record.data)?,
                record.revision,
            ],
        )
        .map_err(map_sqlite_error)?;

        tx.execute(
            "UPDATE sync_revision SET revision = MAX(revision, ?)",
            params![record.revision],
        )
        .map_err(map_sqlite_error)?;

        tx.commit().map_err(map_sqlite_error)?;
        Ok(())
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection
            .prepare(
                "SELECT o.record_type
            ,       o.data_id
            ,       o.schema_version
            ,       o.commit_time
            ,       o.updated_fields_json
            ,       o.revision
            ,       e.schema_version AS existing_schema_version
            ,       e.commit_time AS existing_commit_time
            ,       e.data AS existing_data
            ,       e.revision AS existing_revision
             FROM sync_outgoing o
             LEFT JOIN sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id
             ORDER BY o.revision ASC
             LIMIT ?",
            )
            .map_err(map_sqlite_error)?;
        let mut rows = stmt.query(params![limit]).map_err(map_sqlite_error)?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqlite_error)? {
            let parent = if let Some(existing_data) =
                row.get::<_, Option<String>>(8).map_err(map_sqlite_error)?
            {
                Some(Record {
                    id: RecordId::new(
                        row.get::<_, String>(0).map_err(map_sqlite_error)?,
                        row.get::<_, String>(1).map_err(map_sqlite_error)?,
                    ),
                    schema_version: row.get(6).map_err(map_sqlite_error)?,
                    revision: row.get(9).map_err(map_sqlite_error)?,
                    data: serde_json::from_str(&existing_data)?,
                })
            } else {
                None
            };
            let change = RecordChange {
                id: RecordId::new(
                    row.get::<_, String>(0).map_err(map_sqlite_error)?,
                    row.get::<_, String>(1).map_err(map_sqlite_error)?,
                ),
                schema_version: row.get(2).map_err(map_sqlite_error)?,
                updated_fields: serde_json::from_str(
                    &row.get::<_, String>(4).map_err(map_sqlite_error)?,
                )?,
                local_revision: row.get(5).map_err(map_sqlite_error)?,
            };
            results.push(OutgoingChange { change, parent });
        }

        Ok(results)
    }

    async fn get_last_revision(&self) -> Result<u64, StorageError> {
        let connection = self.get_connection()?;

        let revision: u64 = connection
            .query_row("SELECT revision FROM sync_revision", [], |row| row.get(0))
            .map_err(map_sqlite_error)?;

        Ok(revision)
    }

    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), StorageError> {
        if records.is_empty() {
            return Ok(());
        }

        let mut connection = self.get_connection()?;
        let tx = connection.transaction().map_err(map_sqlite_error)?;

        for record in records {
            tx.execute(
                "INSERT OR REPLACE INTO sync_incoming (
                    record_type
                ,   data_id
                ,   schema_version
                ,   commit_time
                ,   data
                ,   revision
                )
                 VALUES (?, ?, ?, strftime('%s','now'), ?, ?)",
                params![
                    record.id.r#type,
                    record.id.data_id,
                    record.schema_version.clone(),
                    serde_json::to_string(&record.data)?,
                    record.revision,
                ],
            )
            .map_err(map_sqlite_error)?;
        }

        tx.commit().map_err(map_sqlite_error)?;
        Ok(())
    }

    async fn delete_incoming_record(&self, record: Record) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection
            .execute(
                "DELETE FROM sync_incoming WHERE record_type = ? AND data_id = ? AND revision = ?",
                params![record.id.r#type, record.id.data_id, record.revision],
            )
            .map_err(map_sqlite_error)?;

        Ok(())
    }

    async fn get_incoming_records(&self, limit: u32) -> Result<Vec<IncomingChange>, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection
            .prepare(
                "SELECT i.record_type
            ,       i.data_id
            ,       i.schema_version
            ,       i.data
            ,       i.revision
            ,       e.schema_version AS existing_schema_version
            ,       e.commit_time AS existing_commit_time
            ,       e.data AS existing_data
            ,       e.revision AS existing_revision
             FROM sync_incoming i
             LEFT JOIN sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id
             ORDER BY i.revision ASC
             LIMIT ?",
            )
            .map_err(map_sqlite_error)?;

        let mut rows = stmt.query(params![limit]).map_err(map_sqlite_error)?;
        let mut results = Vec::new();

        while let Some(row) = rows.next().map_err(map_sqlite_error)? {
            let parent = if let Some(existing_data) =
                row.get::<_, Option<String>>(7).map_err(map_sqlite_error)?
            {
                Some(Record {
                    id: RecordId::new(
                        row.get::<_, String>(0).map_err(map_sqlite_error)?,
                        row.get::<_, String>(1).map_err(map_sqlite_error)?,
                    ),
                    schema_version: row.get(5).map_err(map_sqlite_error)?,
                    revision: row.get(8).map_err(map_sqlite_error)?,
                    data: serde_json::from_str(&existing_data)?,
                })
            } else {
                None
            };
            let record = Record {
                id: RecordId::new(
                    row.get::<_, String>(0).map_err(map_sqlite_error)?,
                    row.get::<_, String>(1).map_err(map_sqlite_error)?,
                ),
                schema_version: row.get(2).map_err(map_sqlite_error)?,
                data: serde_json::from_str(&row.get::<_, String>(3).map_err(map_sqlite_error)?)?,
                revision: row.get(4).map_err(map_sqlite_error)?,
            };
            results.push(IncomingChange {
                new_state: record,
                old_state: parent,
            });
        }

        Ok(results)
    }

    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection
            .prepare(
                "SELECT o.record_type
            ,       o.data_id
            ,       o.schema_version
            ,       o.commit_time
            ,       o.updated_fields_json
            ,       o.revision
            ,       e.schema_version AS existing_schema_version
            ,       e.commit_time AS existing_commit_time
            ,       e.data AS existing_data
            ,       e.revision AS existing_revision
             FROM sync_outgoing o
             LEFT JOIN sync_state e ON o.record_type = e.record_type AND o.data_id = e.data_id
             ORDER BY o.revision DESC
             LIMIT 1",
            )
            .map_err(map_sqlite_error)?;

        let mut rows = stmt.query([]).map_err(map_sqlite_error)?;

        if let Some(row) = rows.next().map_err(map_sqlite_error)? {
            let parent = if let Some(existing_data) =
                row.get::<_, Option<String>>(8).map_err(map_sqlite_error)?
            {
                Some(Record {
                    id: RecordId::new(
                        row.get::<_, String>(0).map_err(map_sqlite_error)?,
                        row.get::<_, String>(1).map_err(map_sqlite_error)?,
                    ),
                    schema_version: row.get(6).map_err(map_sqlite_error)?,
                    revision: row.get(9).map_err(map_sqlite_error)?,
                    data: serde_json::from_str(&existing_data)?,
                })
            } else {
                None
            };
            let change = RecordChange {
                id: RecordId::new(
                    row.get::<_, String>(0).map_err(map_sqlite_error)?,
                    row.get::<_, String>(1).map_err(map_sqlite_error)?,
                ),
                schema_version: row.get(2).map_err(map_sqlite_error)?,
                updated_fields: serde_json::from_str(
                    &row.get::<_, String>(4).map_err(map_sqlite_error)?,
                )?,
                local_revision: row.get(5).map_err(map_sqlite_error)?,
            };

            return Ok(Some(OutgoingChange { change, parent }));
        }

        Ok(None)
    }

    async fn update_record_from_incoming(&self, record: Record) -> Result<(), StorageError> {
        let mut connection = self.get_connection()?;
        let tx = connection.transaction().map_err(map_sqlite_error)?;

        tx.execute(
            "INSERT OR REPLACE INTO sync_state (
                record_type
            ,   data_id
            ,   schema_version
            ,   commit_time
            ,   data
            ,   revision
            )
             VALUES (?, ?, ?, strftime('%s','now'), ?, ?)",
            params![
                record.id.r#type,
                record.id.data_id,
                record.schema_version.clone(),
                serde_json::to_string(&record.data)?,
                record.revision,
            ],
        )
        .map_err(map_sqlite_error)?;

        tx.execute(
            "UPDATE sync_revision SET revision = MAX(revision, ?)",
            params![record.revision],
        )
        .map_err(map_sqlite_error)?;

        tx.commit().map_err(map_sqlite_error)?;
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
fn map_payment(row: &Row<'_>) -> Result<Payment, rusqlite::Error> {
    let withdraw_tx_id: Option<String> = row.get(7)?;
    let deposit_tx_id: Option<String> = row.get(8)?;
    let spark: Option<i32> = row.get(9)?;
    let lightning_invoice: Option<String> = row.get(10)?;
    let token_metadata: Option<String> = row.get(18)?;
    let details = match (
        lightning_invoice,
        withdraw_tx_id,
        deposit_tx_id,
        spark,
        token_metadata,
    ) {
        (Some(invoice), _, _, _, _) => {
            let payment_hash: String = row.get(11)?;
            let destination_pubkey: String = row.get(12)?;
            let description: Option<String> = row.get(13)?;
            let preimage: Option<String> = row.get(14)?;
            let lnurl_pay_info: Option<LnurlPayInfo> = row.get(15)?;
            let lnurl_withdraw_info: Option<LnurlWithdrawInfo> = row.get(16)?;
            let lnurl_nostr_zap_request: Option<String> = row.get(24)?;
            let lnurl_nostr_zap_receipt: Option<String> = row.get(25)?;
            let lnurl_sender_comment: Option<String> = row.get(26)?;
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
            let invoice_details_str: Option<String> = row.get(22)?;
            let invoice_details = invoice_details_str
                .map(|s| serde_json_from_str(&s, 22))
                .transpose()?;
            let htlc_details_str: Option<String> = row.get(23)?;
            let htlc_details = htlc_details_str
                .map(|s| serde_json_from_str(&s, 23))
                .transpose()?;
            let conversion_info_str: Option<String> = row.get(17)?;
            let conversion_info: Option<ConversionInfo> = conversion_info_str
                .map(|s: String| serde_json_from_str(&s, 17))
                .transpose()?;
            Some(PaymentDetails::Spark {
                invoice_details,
                htlc_details,
                conversion_info,
            })
        }
        (_, _, _, _, Some(metadata)) => {
            let tx_type: TokenTransactionType = row.get(20)?;
            let invoice_details_str: Option<String> = row.get(21)?;
            let invoice_details = invoice_details_str
                .map(|s| serde_json_from_str(&s, 21))
                .transpose()?;
            let conversion_info_str: Option<String> = row.get(17)?;
            let conversion_info: Option<ConversionInfo> = conversion_info_str
                .map(|s: String| serde_json_from_str(&s, 17))
                .transpose()?;
            Some(PaymentDetails::Token {
                metadata: serde_json_from_str(&metadata, 18)?,
                tx_hash: row.get(19)?,
                tx_type,
                invoice_details,
                conversion_info,
            })
        }
        _ => None,
    };
    Ok(Payment {
        id: row.get(0)?,
        payment_type: row.get::<_, String>(1)?.parse().map_err(|e: String| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
        })?,
        status: row.get::<_, String>(2)?.parse().map_err(|e: String| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, e.into())
        })?,
        amount: row.get::<_, U128SqlWrapper>(3)?.0,
        fees: row.get::<_, U128SqlWrapper>(4)?.0,
        timestamp: row.get(5)?,
        details,
        method: row.get(6)?,
        conversion_details: None,
    })
}

impl ToSql for PaymentDetails {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        to_sql_json(self)
    }
}

impl FromSql for PaymentDetails {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        from_sql_json(value)
    }
}

impl ToSql for PaymentMethod {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.to_string()))
    }
}

impl FromSql for PaymentMethod {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                // NOTE: trim_matches/to_lowercase is here, because this used to be serde_json serialized.
                let payment_method: PaymentMethod = s
                    .trim_matches('"')
                    .to_lowercase()
                    .parse()
                    .map_err(|()| FromSqlError::InvalidType)?;
                Ok(payment_method)
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for TokenTransactionType {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.to_string()))
    }
}

impl FromSql for TokenTransactionType {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                let tx_type: TokenTransactionType =
                    s.parse().map_err(|_: String| FromSqlError::InvalidType)?;
                Ok(tx_type)
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for DepositClaimError {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        to_sql_json(self)
    }
}

impl FromSql for DepositClaimError {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        from_sql_json(value)
    }
}

impl ToSql for LnurlPayInfo {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        to_sql_json(self)
    }
}

impl FromSql for LnurlPayInfo {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        from_sql_json(value)
    }
}

impl ToSql for LnurlWithdrawInfo {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        to_sql_json(self)
    }
}

impl FromSql for LnurlWithdrawInfo {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        from_sql_json(value)
    }
}

fn to_sql_json<T>(value: T) -> rusqlite::Result<ToSqlOutput<'static>>
where
    T: serde::Serialize,
{
    let json = serde_json::to_string(&value)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(rusqlite::types::ToSqlOutput::from(json))
}

fn from_sql_json<T>(value: ValueRef<'_>) -> FromSqlResult<T>
where
    T: serde::de::DeserializeOwned,
{
    match value {
        ValueRef::Text(i) => {
            let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
            let deserialized: T = serde_json::from_str(s).map_err(|_| FromSqlError::InvalidType)?;
            Ok(deserialized)
        }
        _ => Err(FromSqlError::InvalidType),
    }
}

fn serde_json_from_str<T>(value: &str, index: usize) -> Result<T, rusqlite::Error>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

struct U128SqlWrapper(u128);

impl ToSql for U128SqlWrapper {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let string = self.0.to_string();
        Ok(rusqlite::types::ToSqlOutput::from(string))
    }
}

impl FromSql for U128SqlWrapper {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                let integer = s.parse::<u128>().map_err(|_| FromSqlError::InvalidType)?;
                Ok(U128SqlWrapper(integer))
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::SqliteStorage;
    use std::path::PathBuf;

    /// Helper function to create a temporary directory for tests
    /// Uses std library to avoid external dependency
    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        // Use UUID for uniqueness to avoid conflicts between parallel tests
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn test_storage() {
        let temp_dir = create_temp_dir("sqlite_storage");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        Box::pin(crate::persist::tests::test_storage(Box::new(storage))).await;
    }

    #[tokio::test]
    async fn test_unclaimed_deposits_crud() {
        let temp_dir = create_temp_dir("sqlite_storage_deposits");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_deposit_refunds() {
        let temp_dir = create_temp_dir("sqlite_storage_refund_tx");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_deposit_refunds(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_payment_type_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_type_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_payment_type_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_payment_status_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_status_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_payment_status_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_payment_asset_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_asset_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_asset_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_timestamp_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_timestamp_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_timestamp_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_spark_htlc_status_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_htlc_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_spark_htlc_status_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_conversion_refund_needed_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_conversion_refund_needed_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_conversion_refund_needed_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_token_transaction_type_filtering() {
        let temp_dir = create_temp_dir("sqlite_storage_token_transaction_type_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_token_transaction_type_filtering(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_combined_filters() {
        let temp_dir = create_temp_dir("sqlite_storage_combined_filter");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_combined_filters(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_sort_order() {
        let temp_dir = create_temp_dir("sqlite_storage_sort_order");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_sort_order(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_payment_metadata() {
        let temp_dir = create_temp_dir("sqlite_storage_payment_request_metadata");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_payment_metadata(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_payment_details_update_persistence() {
        let temp_dir = create_temp_dir("sqlite_storage_payment_details_update");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_payment_details_update_persistence(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_sync_storage() {
        let temp_dir = create_temp_dir("sqlite_sync_storage");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_sync_storage(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_payment_metadata_merge() {
        let temp_dir = create_temp_dir("sqlite_payment_metadata_merge");
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        crate::persist::tests::test_payment_metadata_merge(Box::new(storage)).await;
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_migration_tx_type() {
        use crate::{
            ListPaymentsRequest, Payment, PaymentDetails, PaymentDetailsFilter, PaymentMethod,
            PaymentStatus, PaymentType, Storage, TokenMetadata, TokenTransactionType,
        };
        use rusqlite::{Connection, params};
        use rusqlite_migration::{M, Migrations};

        let temp_dir = create_temp_dir("sqlite_migration_tx_type");
        let db_path = temp_dir.join(super::DEFAULT_DB_FILENAME);

        // Step 1: Create database at version 21 (before tx_type migration)
        {
            let mut conn = Connection::open(&db_path).unwrap();
            let migrations_before_tx_type: Vec<_> = SqliteStorage::current_migrations()
                .iter()
                .take(22) // Migrations 0-21 (index 22 is the tx_type migration)
                .map(|s| M::up(s))
                .collect();
            let migrations = Migrations::new(migrations_before_tx_type);
            migrations.to_latest(&mut conn).unwrap();
        }

        // Step 2: Insert a token payment WITHOUT tx_type column
        {
            let conn = Connection::open(&db_path).unwrap();

            // Insert into payments table
            conn.execute(
                "INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    "token-migration-test",
                    "send",
                    "completed",
                    "5000",
                    "10",
                    1_234_567_890_i64,
                    "\"token\""
                ],
            )
            .unwrap();

            // Insert into payment_details_token WITHOUT tx_type (pre-migration)
            let metadata = serde_json::json!({
                "identifier": "test-token-id",
                "issuer_public_key": format!("02{}", "a".repeat(64)),
                "name": "Test Token",
                "ticker": "TST",
                "decimals": 8,
                "max_supply": 1_000_000_u128,
                "is_freezable": false
            });

            conn.execute(
                "INSERT INTO payment_details_token (payment_id, metadata, tx_hash)
                 VALUES (?, ?, ?)",
                params![
                    "token-migration-test",
                    metadata.to_string(),
                    "0xabcdef1234567890"
                ],
            )
            .unwrap();
        }

        // Step 3: Open with SqliteStorage (triggers migration to latest)
        let storage = SqliteStorage::new(&temp_dir).unwrap();

        // Step 4: Verify the migrated token payment
        let migrated_payment = storage
            .get_payment_by_id("token-migration-test".to_string())
            .await
            .unwrap();

        assert_eq!(migrated_payment.id, "token-migration-test");
        assert_eq!(migrated_payment.amount, 5000);
        assert_eq!(migrated_payment.fees, 10);
        assert_eq!(migrated_payment.status, PaymentStatus::Completed);
        assert_eq!(migrated_payment.payment_type, PaymentType::Send);
        assert_eq!(migrated_payment.method, PaymentMethod::Token);

        // Verify token payment details have the default txType
        match migrated_payment.details {
            Some(PaymentDetails::Token {
                metadata,
                tx_hash,
                tx_type,
                ..
            }) => {
                assert_eq!(metadata.identifier, "test-token-id");
                assert_eq!(metadata.name, "Test Token");
                assert_eq!(metadata.ticker, "TST");
                assert_eq!(metadata.decimals, 8);
                assert_eq!(tx_hash, "0xabcdef1234567890");
                // Key assertion: migration added default tx_type
                assert_eq!(
                    tx_type,
                    TokenTransactionType::Transfer,
                    "Migration should add default txType 'transfer' to token payments"
                );
            }
            _ => panic!("Expected Token payment details"),
        }

        // Step 5: Insert a new token payment with explicit tx_type
        let new_payment = Payment {
            id: "new-token-payment".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 8000,
            fees: 20,
            timestamp: 1_234_567_891,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: TokenMetadata {
                    identifier: "another-token-id".to_string(),
                    issuer_public_key: format!("02{}", "b".repeat(64)),
                    name: "Another Token".to_string(),
                    ticker: "ATK".to_string(),
                    decimals: 6,
                    max_supply: 2_000_000,
                    is_freezable: true,
                },
                tx_hash: "0x1111222233334444".to_string(),
                tx_type: TokenTransactionType::Mint,
                invoice_details: None,
                conversion_info: None,
            }),
            conversion_details: None,
        };

        storage.insert_payment(new_payment).await.unwrap();

        // Step 6: List all payments
        let request = ListPaymentsRequest {
            type_filter: None,
            status_filter: None,
            asset_filter: None,
            payment_details_filter: None,
            from_timestamp: None,
            to_timestamp: None,
            offset: None,
            limit: None,
            sort_ascending: Some(true),
        };

        let payments = storage.list_payments(request).await.unwrap();
        assert_eq!(payments.len(), 2, "Should have both payments");

        // Verify migrated payment has Transfer type
        let migrated = payments
            .iter()
            .find(|p| p.id == "token-migration-test")
            .unwrap();
        match &migrated.details {
            Some(PaymentDetails::Token { tx_type, .. }) => {
                assert_eq!(*tx_type, TokenTransactionType::Transfer);
            }
            _ => panic!("Expected Token payment details"),
        }

        // Verify new payment has Mint type
        let new = payments
            .iter()
            .find(|p| p.id == "new-token-payment")
            .unwrap();
        match &new.details {
            Some(PaymentDetails::Token { tx_type, .. }) => {
                assert_eq!(*tx_type, TokenTransactionType::Mint);
            }
            _ => panic!("Expected Token payment details"),
        }

        // Step 7: Test filtering by token transaction type
        let transfer_filter = ListPaymentsRequest {
            type_filter: None,
            status_filter: None,
            asset_filter: None,
            payment_details_filter: Some(vec![PaymentDetailsFilter::Token {
                conversion_refund_needed: None,
                tx_hash: None,
                tx_type: Some(TokenTransactionType::Transfer),
            }]),
            from_timestamp: None,
            to_timestamp: None,
            offset: None,
            limit: None,
            sort_ascending: Some(true),
        };

        let transfer_payments = storage.list_payments(transfer_filter).await.unwrap();
        assert_eq!(
            transfer_payments.len(),
            1,
            "Should find only the Transfer payment"
        );
        assert_eq!(transfer_payments[0].id, "token-migration-test");
    }
}
