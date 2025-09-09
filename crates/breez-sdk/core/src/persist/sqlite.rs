use macros::async_trait;
use rusqlite::{
    Connection, ToSql, params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
};
use rusqlite_migration::{M, Migrations};
use std::path::{Path, PathBuf};

use crate::{
    DepositInfo, LnurlPayInfo, PaymentDetails, PaymentMethod,
    error::DepositClaimError,
    models::{PaymentStatus, PaymentType},
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
        migrations.to_latest(&mut conn)?;
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
              lnurl_pay_info TEXT);",
            "CREATE TABLE IF NOT EXISTS deposit_refunds (
              deposit_tx_id TEXT NOT NULL,
              deposit_vout INTEGER NOT NULL,
              refund_tx TEXT NOT NULL,
              refund_tx_id TEXT NOT NULL,
              PRIMARY KEY (deposit_tx_id, deposit_vout)              
            );",
            // Migration to change payments amount and fees from INTEGER to TEXT
            "CREATE TABLE payments_new (
              id TEXT PRIMARY KEY,
              payment_type TEXT NOT NULL,
              status TEXT NOT NULL,
              amount TEXT NOT NULL,
              fees TEXT NOT NULL,
              timestamp INTEGER NOT NULL,
              details TEXT,
              method TEXT
            );",
            "INSERT INTO payments_new (id, payment_type, status, amount, fees, timestamp, details, method)
             SELECT id, payment_type, status, CAST(amount AS TEXT), CAST(fees AS TEXT), timestamp, details, method
             FROM payments;",
            "DROP TABLE payments;",
            "ALTER TABLE payments_new RENAME TO payments;",
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
            "SELECT p.id, p.payment_type, p.status, p.amount, p.fees, p.timestamp, p.details, p.method, pm.lnurl_pay_info
             FROM payments p
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             ORDER BY p.timestamp DESC 
             LIMIT {} OFFSET {}",
            limit.unwrap_or(u32::MAX),
            offset.unwrap_or(0)
        );

        let mut stmt = connection.prepare(&query)?;

        let payment_iter = stmt.query_map(params![], |row| {
            let mut details: Option<PaymentDetails> = row.get(6)?;
            if let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = &mut details {
                *lnurl_pay_info = row.get(8)?;
            }

            Ok(Payment {
                id: row.get(0)?,
                payment_type: PaymentType::from(row.get::<_, String>(1)?.as_str()),
                status: PaymentStatus::from(row.get::<_, String>(2)?.as_str()),
                amount: row.get::<_, U128SqlWrapper>(3)?.0,
                fees: row.get::<_, U128SqlWrapper>(4)?.0,
                timestamp: row.get(5)?,
                details,
                method: row.get(7)?,
            })
        })?;

        let mut payments = Vec::new();
        for payment in payment_iter {
            payments.push(payment?);
        }

        Ok(payments)
    }

    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO payments (id, payment_type, status, amount, fees, timestamp, details, method) 
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                payment.id,
                payment.payment_type.to_string(),
                payment.status.to_string(),
                U128SqlWrapper(payment.amount),
                U128SqlWrapper(payment.fees),
                payment.timestamp,
                payment.details,
                payment.method,
            ],
        )?;

        Ok(())
    }

    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO payment_metadata (payment_id, lnurl_pay_info) VALUES (?, ?)",
            params![payment_id, metadata.lnurl_pay_info],
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

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection.prepare(
            "SELECT id, payment_type, status, amount, fees, timestamp, details, method, pm.lnurl_pay_info FROM payments LEFT JOIN payment_metadata pm ON payments.id = pm.payment_id WHERE payments.id = ?",
        )?;

        let result = stmt.query_row(params![id], |row| {
            let mut details: Option<PaymentDetails> = row.get(6)?;
            if let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = &mut details {
                *lnurl_pay_info = row.get(8)?;
            }

            Ok(Payment {
                id: row.get(0)?,
                payment_type: PaymentType::from(row.get::<_, String>(1)?.as_str()),
                status: PaymentStatus::from(row.get::<_, String>(2)?.as_str()),
                amount: row.get::<_, U128SqlWrapper>(3)?.0,
                fees: row.get::<_, U128SqlWrapper>(4)?.0,
                timestamp: row.get(5)?,
                details,
                method: row.get(7)?,
            })
        });
        result.map_err(StorageError::from)
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
                let s = std::str::from_utf8(i).map_err(FromSqlError::other)?;
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
        let json = serde_json::to_string(self)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(rusqlite::types::ToSqlOutput::from(json))
    }
}

impl FromSql for PaymentMethod {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(i) => {
                let s = std::str::from_utf8(i).map_err(FromSqlError::other)?;
                let payment_method: PaymentMethod =
                    serde_json::from_str(s).map_err(|_| FromSqlError::InvalidType)?;
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
                let s = std::str::from_utf8(i).map_err(FromSqlError::other)?;
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
                let s = std::str::from_utf8(i).map_err(FromSqlError::other)?;
                let lnurl_pay_info: LnurlPayInfo =
                    serde_json::from_str(s).map_err(|_| FromSqlError::InvalidType)?;
                Ok(lnurl_pay_info)
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
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
                let s = std::str::from_utf8(i).map_err(FromSqlError::other)?;
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
