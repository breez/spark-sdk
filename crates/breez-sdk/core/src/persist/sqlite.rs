use rusqlite::{
    Connection, ToSql, params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
};
use rusqlite_migration::{M, Migrations};
use std::path::{Path, PathBuf};

use crate::{
    DepositInfo, DepositRefund, LnurlPayInfo, PaymentDetails,
    error::DepositClaimError,
    models::{PaymentStatus, PaymentType},
    persist::PaymentMetadata,
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
              timestamp INTEGER NOT NULL
            );",
            "CREATE TABLE IF NOT EXISTS settings (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );",
            "ALTER TABLE payments ADD COLUMN details TEXT;",
            "CREATE TABLE IF NOT EXISTS unclaimed_deposits (
              txid TEXT NOT NULL,
              vout INTEGER NOT NULL,
              amount_sats INTEGER,
              error TEXT,
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

impl Storage for SqliteStorage {
    fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, StorageError> {
        let connection = self.get_connection()?;

        let query = format!(
            "SELECT p.id, p.payment_type, p.status, p.amount, p.fees, p.timestamp, p.details, pm.lnurl_pay_info
             FROM payments p
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             ORDER BY p.timestamp DESC 
             LIMIT {} OFFSET {}",
            limit.unwrap_or(u32::MAX),
            offset.unwrap_or(0)
        );

        let mut stmt = connection.prepare(&query)?;

        let payment_iter = stmt.query_map(params![], |row| {
            let mut details = row.get(6)?;
            if let PaymentDetails::Lightning { lnurl_pay_info, .. } = &mut details {
                *lnurl_pay_info = row.get(7)?;
            }

            Ok(Payment {
                id: row.get(0)?,
                payment_type: PaymentType::from(row.get::<_, String>(1)?.as_str()),
                status: PaymentStatus::from(row.get::<_, String>(2)?.as_str()),
                amount: row.get(3)?,
                fees: row.get(4)?,
                timestamp: row.get(5)?,
                details,
            })
        })?;

        let mut payments = Vec::new();
        for payment in payment_iter {
            payments.push(payment?);
        }

        Ok(payments)
    }

    fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO payments (id, payment_type, status, amount, fees, timestamp, details) 
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                payment.id,
                payment.payment_type.to_string(),
                payment.status.to_string(),
                payment.amount,
                payment.fees,
                payment.timestamp,
                payment.details,
            ],
        )?;

        Ok(())
    }

    fn set_payment_metadata(
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

    fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
            params![key, value],
        )?;

        Ok(())
    }

    fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
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

    fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection.prepare(
            "SELECT id, payment_type, status, amount, fees, timestamp, details FROM payments WHERE id = ?",
        )?;

        let result = stmt.query_row(params![id], |row| {
            Ok(Payment {
                id: row.get(0)?,
                payment_type: PaymentType::from(row.get::<_, String>(1)?.as_str()),
                status: PaymentStatus::from(row.get::<_, String>(2)?.as_str()),
                amount: row.get(3)?,
                fees: row.get(4)?,
                timestamp: row.get(5)?,
                details: row.get(6)?,
            })
        });
        result.map_err(StorageError::from)
    }

    fn add_unclaimed_deposit(&self, deposit_info: DepositInfo) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        connection.execute(
            "INSERT OR REPLACE INTO unclaimed_deposits (txid, vout, amount_sats, error) 
             VALUES (?, ?, ?, ?)",
            params![
                deposit_info.txid,
                deposit_info.vout,
                deposit_info.amount_sats,
                deposit_info.error,
            ],
        )?;
        Ok(())
    }

    fn remove_unclaimed_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        connection.execute(
            "DELETE FROM unclaimed_deposits WHERE txid = ? AND vout = ?",
            params![txid, vout],
        )?;
        Ok(())
    }

    fn list_unclaimed_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let connection = self.get_connection()?;
        let mut stmt =
            connection.prepare("SELECT txid, vout, amount_sats, error FROM unclaimed_deposits")?;
        let rows = stmt.query_map(params![], |row| {
            Ok(DepositInfo {
                txid: row.get(0)?,
                vout: row.get(1)?,
                amount_sats: row.get(2)?,
                error: row.get(3)?,
            })
        })?;
        let mut deposits = Vec::new();
        for row in rows {
            deposits.push(row?);
        }
        Ok(deposits)
    }

    fn set_unclaimed_deposits(&self, deposits: Vec<DepositInfo>) -> Result<(), StorageError> {
        let mut connection = self.get_connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM unclaimed_deposits", params![])?;
        for deposit in deposits {
            transaction.execute(
                "INSERT OR REPLACE INTO unclaimed_deposits (txid, vout, amount_sats, error) 
                 VALUES (?, ?, ?, ?)",
                params![
                    deposit.txid,
                    deposit.vout,
                    deposit.amount_sats,
                    deposit.error,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn update_deposit_refund(&self, deposit_refund: DepositRefund) -> Result<(), StorageError> {
        let connection = self.get_connection()?;
        connection.execute(
            "INSERT OR REPLACE INTO deposit_refunds (deposit_tx_id, deposit_vout, refund_tx, refund_tx_id) 
             VALUES (?, ?, ?, ?)",
            params![
                deposit_refund.deposit_tx_id,
                deposit_refund.deposit_vout,
                deposit_refund.refund_tx,
                deposit_refund.refund_tx_id,
            ],
        )?;
        Ok(())
    }

    fn get_deposit_refund(
        &self,
        txid: String,
        vout: u32,
    ) -> Result<Option<DepositRefund>, StorageError> {
        let connection = self.get_connection()?;
        let mut stmt = connection.prepare(
            "SELECT deposit_tx_id, deposit_vout, refund_tx, refund_tx_id FROM deposit_refunds WHERE deposit_tx_id = ? AND deposit_vout = ?",
        )?;
        let result = stmt.query_row(params![txid, vout], |row| {
            Ok(DepositRefund {
                deposit_tx_id: row.get(0)?,
                deposit_vout: row.get(1)?,
                refund_tx: row.get(2)?,
                refund_tx_id: row.get(3)?,
            })
        });
        match result {
            Ok(deposit_refund) => Ok(Some(deposit_refund)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_sqlite_storage() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        // Create test payment
        let payment = Payment {
            id: "pmt123".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 100_000,
            fees: 1000,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            details: PaymentDetails::Spark,
        };

        // Insert payment
        storage.insert_payment(payment.clone()).unwrap();

        // List payments
        let payments = storage.list_payments(Some(0), Some(10)).unwrap();
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].id, payment.id);
        assert_eq!(payments[0].payment_type, payment.payment_type);
        assert_eq!(payments[0].status, payment.status);
        assert_eq!(payments[0].amount, payment.amount);
        assert_eq!(payments[0].fees, payment.fees);
        assert!(matches!(payments[0].details, PaymentDetails::Spark));

        // Get payment by ID
        let retrieved_payment = storage.get_payment_by_id(payment.id.clone()).unwrap();
        assert_eq!(retrieved_payment.id, payment.id);
        assert_eq!(retrieved_payment.payment_type, payment.payment_type);
        assert_eq!(retrieved_payment.status, payment.status);
        assert_eq!(retrieved_payment.amount, payment.amount);
        assert_eq!(retrieved_payment.fees, payment.fees);
    }

    #[test]
    fn test_unclaimed_deposits_crud() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_deposits").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        // Create test deposit info
        let deposit_1 = crate::DepositInfo {
            txid: "tx123".to_string(),
            vout: 0,
            amount_sats: Some(50000),
            error: None,
        };

        let deposit_2 = crate::DepositInfo {
            txid: "tx456".to_string(),
            vout: 1,
            amount_sats: Some(75000),
            error: Some(DepositClaimError::Generic {
                message: "Test error".to_string(),
            }),
        };

        // Initially, list should be empty
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 0);

        // Add first deposit
        storage.add_unclaimed_deposit(deposit_1).unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "tx123");
        assert_eq!(deposits[0].vout, 0);
        assert_eq!(deposits[0].amount_sats, Some(50000));
        assert!(deposits[0].error.is_none());

        // Add second deposit
        storage.add_unclaimed_deposit(deposit_2).unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 2);

        // Find deposit2 in the list
        let deposit2_found = deposits.iter().find(|d| d.txid == "tx456").unwrap();
        assert_eq!(deposit2_found.vout, 1);
        assert_eq!(deposit2_found.amount_sats, Some(75000));
        assert!(deposit2_found.error.is_some());

        // Remove first deposit
        storage
            .remove_unclaimed_deposit("tx123".to_string(), 0)
            .unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "tx456");

        // Remove second deposit
        storage
            .remove_unclaimed_deposit("tx456".to_string(), 1)
            .unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 0);
    }

    #[test]
    fn test_set_unclaimed_deposits() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_set_deposits").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        // Create test deposits
        let deposits = vec![
            crate::DepositInfo {
                txid: "tx1".to_string(),
                vout: 0,
                amount_sats: Some(10000),
                error: None,
            },
            crate::DepositInfo {
                txid: "tx2".to_string(),
                vout: 1,
                amount_sats: Some(20000),
                error: Some(DepositClaimError::Generic {
                    message: "Error 1".to_string(),
                }),
            },
            crate::DepositInfo {
                txid: "tx3".to_string(),
                vout: 0,
                amount_sats: None,
                error: Some(DepositClaimError::MissingUtxo {
                    tx: "tx3".to_string(),
                    vout: 0,
                }),
            },
        ];

        // Set deposits (should replace any existing ones)
        storage.set_unclaimed_deposits(deposits).unwrap();
        let stored_deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(stored_deposits.len(), 3);

        // Verify all deposits are stored correctly
        let tx1_deposit = stored_deposits.iter().find(|d| d.txid == "tx1").unwrap();
        assert_eq!(tx1_deposit.vout, 0);
        assert_eq!(tx1_deposit.amount_sats, Some(10000));
        assert!(tx1_deposit.error.is_none());

        let tx2_deposit = stored_deposits.iter().find(|d| d.txid == "tx2").unwrap();
        assert_eq!(tx2_deposit.vout, 1);
        assert_eq!(tx2_deposit.amount_sats, Some(20000));
        assert!(tx2_deposit.error.is_some());

        let tx3_deposit = stored_deposits.iter().find(|d| d.txid == "tx3").unwrap();
        assert_eq!(tx3_deposit.vout, 0);
        assert_eq!(tx3_deposit.amount_sats, None);
        assert!(tx3_deposit.error.is_some());

        // Set with empty list (should clear all deposits)
        storage.set_unclaimed_deposits(Vec::new()).unwrap();
        let stored_deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(stored_deposits.len(), 0);
    }

    #[test]
    fn test_add_unclaimed_deposit_replace() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_replace").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        // Create initial deposit
        let deposit_1 = crate::DepositInfo {
            txid: "tx123".to_string(),
            vout: 0,
            amount_sats: Some(50000),
            error: None,
        };

        // Add deposit
        storage.add_unclaimed_deposit(deposit_1).unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 1);
        assert!(deposits[0].error.is_none());

        // Update same deposit with error (should replace)
        let deposit1_updated = crate::DepositInfo {
            txid: "tx123".to_string(),
            vout: 0,
            amount_sats: Some(50000),
            error: Some(DepositClaimError::Generic {
                message: "Updated error".to_string(),
            }),
        };

        storage.add_unclaimed_deposit(deposit1_updated).unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 1); // Should still be 1 (replaced, not added)
        assert!(deposits[0].error.is_some());
    }

    #[test]
    fn test_remove_nonexistent_deposit() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_remove").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        // Try to remove a deposit that doesn't exist (should not error)
        storage
            .remove_unclaimed_deposit("nonexistent".to_string(), 0)
            .unwrap();

        // List should still be empty
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 0);
    }

    #[test]
    fn test_deposit_refunds_table() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_refund_tx").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).unwrap();

        // Create initial deposit without refund transaction
        let deposit = crate::DepositInfo {
            txid: "test_tx_123".to_string(),
            vout: 0,
            amount_sats: Some(100_000),
            error: None,
        };

        // Add the initial deposit
        storage.add_unclaimed_deposit(deposit).unwrap();
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "test_tx_123");
        assert_eq!(deposits[0].vout, 0);
        assert_eq!(deposits[0].amount_sats, Some(100_000));
        assert!(deposits[0].error.is_none());

        // Add refund transaction details using the new separate table
        let deposit_refund = crate::DepositRefund {
            deposit_tx_id: "test_tx_123".to_string(),
            deposit_vout: 0,
            refund_tx: "0200000001abcd1234...".to_string(),
            refund_tx_id: "refund_tx_id_456".to_string(),
        };

        // Update the deposit refund information
        storage.update_deposit_refund(deposit_refund).unwrap();

        // Verify that the deposit information remains unchanged
        let deposits = storage.list_unclaimed_deposits().unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "test_tx_123");
        assert_eq!(deposits[0].vout, 0);
        assert_eq!(deposits[0].amount_sats, Some(100_000));
        assert!(deposits[0].error.is_none());

        // Verify that refund data is stored separately (would need a query method to fully test)
        // For now, we verify that the update_deposit_refund method doesn't error

        // Test updating the same refund (should replace)
        let updated_refund = crate::DepositRefund {
            deposit_tx_id: "test_tx_123".to_string(),
            deposit_vout: 0,
            refund_tx: "0200000001updated...".to_string(),
            refund_tx_id: "updated_refund_id".to_string(),
        };
        storage.update_deposit_refund(updated_refund).unwrap();
    }
}
