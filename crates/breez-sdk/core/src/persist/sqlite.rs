use macros::async_trait;
use rusqlite::{
    Connection, Row, ToSql, params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
};
use rusqlite_migration::{M, Migrations, SchemaVersion};
use std::path::{Path, PathBuf};

use crate::{
    DepositInfo, LnurlPayInfo, PaymentDetails, PaymentMethod,
    error::DepositClaimError,
    persist::{PaymentMetadata, UpdateDepositPayload},
};

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
        ]
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        StorageError::Implementation(value.to_string())
    }
}

impl From<rusqlite_migration::Error> for StorageError {
    fn from(value: rusqlite_migration::Error) -> Self {
        StorageError::Implementation(value.to_string())
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, StorageError> {
        let connection = self.get_connection()?;

        let query = format!(
            "SELECT p.id
            ,       p.payment_type
            ,       p.status
            ,       p.amount
            ,       p.fees
            ,       p.timestamp
            ,       p.method
            ,       p.withdraw_tx_id
            ,       p.deposit_tx_id
            ,       p.spark
            ,       l.invoice AS lightning_invoice
            ,       l.payment_hash AS lightning_payment_hash
            ,       l.destination_pubkey AS lightning_destination_pubkey
            ,       COALESCE(l.description, pm.lnurl_description) AS lightning_description
            ,       l.preimage AS lightning_preimage
            ,       pm.lnurl_pay_info
             FROM payments p
             LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             ORDER BY p.timestamp DESC 
             LIMIT {} OFFSET {}",
            limit.unwrap_or(u32::MAX),
            offset.unwrap_or(0)
        );

        let mut stmt = connection.prepare(&query)?;
        let payments = stmt
            .query_map(params![], map_payment)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(payments)
    }

    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let mut connection = self.get_connection()?;
        let tx = connection.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO payments (id, payment_type, status, amount, fees, timestamp, method) 
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                payment.id,
                payment.payment_type.to_string(),
                payment.status.to_string(),
                payment.amount,
                payment.fees,
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
            Some(PaymentDetails::Spark) => {
                tx.execute(
                    "UPDATE payments SET spark = 1 WHERE id = ?",
                    params![payment.id],
                )?;
            }
            Some(PaymentDetails::Lightning {
                invoice,
                payment_hash,
                destination_pubkey,
                description,
                preimage,
                lnurl_pay_info: _,
            }) => {
                tx.execute(
                    "INSERT OR REPLACE INTO payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, description, preimage) 
                     VALUES (?, ?, ?, ?, ?, ?)",
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

    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO payment_metadata (payment_id, lnurl_pay_info, lnurl_description) VALUES (?, ?, ?)",
            params![payment_id, metadata.lnurl_pay_info, metadata.lnurl_description],
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

        let mut stmt = connection.prepare(
            "SELECT p.id
            ,       p.payment_type
            ,       p.status
            ,       p.amount
            ,       p.fees
            ,       p.timestamp
            ,       p.method
            ,       p.withdraw_tx_id
            ,       p.deposit_tx_id
            ,       p.spark
            ,       l.invoice AS lightning_invoice
            ,       l.payment_hash AS lightning_payment_hash
            ,       l.destination_pubkey AS lightning_destination_pubkey
            ,       COALESCE(l.description, pm.lnurl_description) AS lightning_description
            ,       l.preimage AS lightning_preimage
            ,       pm.lnurl_pay_info
             FROM payments p
             LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             WHERE p.id = ?",
        )?;

        let payment = stmt.query_row(params![id], map_payment)?;
        Ok(payment)
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection.prepare(
            "SELECT p.id
            ,       p.payment_type
            ,       p.status
            ,       p.amount
            ,       p.fees
            ,       p.timestamp
            ,       p.method
            ,       p.withdraw_tx_id
            ,       p.deposit_tx_id
            ,       p.spark
            ,       l.invoice AS lightning_invoice
            ,       l.payment_hash AS lightning_payment_hash
            ,       l.destination_pubkey AS lightning_destination_pubkey
            ,       COALESCE(l.description, pm.lnurl_description) AS lightning_description
            ,       l.preimage AS lightning_preimage
            ,       pm.lnurl_pay_info
             FROM payments p
             LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             WHERE l.invoice = ?",
        )?;

        let payment = stmt.query_row(params![invoice], map_payment);
        match payment {
            Ok(payment) => Ok(Some(payment)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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
}

fn map_payment(row: &Row<'_>) -> Result<Payment, rusqlite::Error> {
    let withdraw_tx_id: Option<String> = row.get(7)?;
    let deposit_tx_id: Option<String> = row.get(8)?;
    let spark: Option<i32> = row.get(9)?;
    let lightning_invoice: Option<String> = row.get(10)?;
    let details = match (lightning_invoice, withdraw_tx_id, deposit_tx_id, spark) {
        (Some(invoice), _, _, _) => {
            let payment_hash: String = row.get(11)?;
            let destination_pubkey: String = row.get(12)?;
            let description: Option<String> = row.get(13)?;
            let preimage: Option<String> = row.get(14)?;
            let lnurl_pay_info: Option<LnurlPayInfo> = row.get(15)?;

            Some(PaymentDetails::Lightning {
                invoice,
                payment_hash,
                destination_pubkey,
                description,
                preimage,
                lnurl_pay_info,
            })
        }
        (_, Some(tx_id), _, _) => Some(PaymentDetails::Withdraw { tx_id }),
        (_, _, Some(tx_id), _) => Some(PaymentDetails::Deposit { tx_id }),
        (_, _, _, Some(_)) => Some(PaymentDetails::Spark),
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
        amount: row.get(3)?,
        fees: row.get(4)?,
        timestamp: row.get(5)?,
        details,
        method: row.get(6)?,
    })
}

impl ToSql for PaymentDetails {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let json = serde_json::to_string(self)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(rusqlite::types::ToSqlOutput::from(json))
    }
}

impl FromSql for PaymentDetails {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                let payment_details: PaymentDetails =
                    serde_json::from_str(s).map_err(|_| FromSqlError::InvalidType)?;
                Ok(payment_details)
            }
            _ => Err(FromSqlError::InvalidType),
        }
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

impl ToSql for DepositClaimError {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let json = serde_json::to_string(self)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(rusqlite::types::ToSqlOutput::from(json))
    }
}

impl ToSql for LnurlPayInfo {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let json = serde_json::to_string(self)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(rusqlite::types::ToSqlOutput::from(json))
    }
}

impl FromSql for DepositClaimError {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                let deposit_claim_error: DepositClaimError =
                    serde_json::from_str(s).map_err(|_| FromSqlError::InvalidType)?;
                Ok(deposit_claim_error)
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl FromSql for LnurlPayInfo {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                let lnurl_pay_info: LnurlPayInfo =
                    serde_json::from_str(s).map_err(|_| FromSqlError::InvalidType)?;
                Ok(lnurl_pay_info)
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::SqliteStorage;

    #[tokio::test]
    async fn test_sqlite_storage() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        crate::persist::tests::test_sqlite_storage(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_unclaimed_deposits_crud() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_deposits").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        crate::persist::tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_deposit_refunds() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_refund_tx").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        crate::persist::tests::test_deposit_refunds(Box::new(storage)).await;
    }
}
