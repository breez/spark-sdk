use rusqlite::{
    Connection, ToSql, params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
};
use rusqlite_migration::{M, Migrations};
use std::path::{Path, PathBuf};

use crate::PaymentDetails;

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
            "SELECT id, payment_type, status, amount, fees, timestamp, details FROM payments ORDER BY timestamp DESC LIMIT {} OFFSET {}",
            limit.unwrap_or(u32::MAX),
            offset.unwrap_or(0)
        );

        let mut stmt = connection.prepare(&query)?;

        let payments = stmt
            .query_map(params![], |row| {
                Ok(Payment {
                    id: row.get(0)?,
                    payment_type: row.get::<_, String>(1)?.parse().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            "Failed to parse payment type".into(),
                        )
                    })?,
                    status: row.get::<_, String>(2)?.parse().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            "Failed to parse payment status".into(),
                        )
                    })?,
                    amount: row.get(3)?,
                    fees: row.get(4)?,
                    timestamp: row.get(5)?,
                    details: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(payments)
    }

    fn insert_payment(&self, payment: &Payment) -> Result<(), StorageError> {
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

    fn set_cached_item(&self, key: &str, value: String) -> Result<(), StorageError> {
        let connection = self.get_connection()?;

        connection.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
            params![key, value],
        )?;

        Ok(())
    }

    fn get_cached_item(&self, key: &str) -> Result<Option<String>, StorageError> {
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

    fn get_payment_by_id(&self, id: &str) -> Result<Payment, StorageError> {
        let connection = self.get_connection()?;

        let mut stmt = connection.prepare(
            "SELECT id, payment_type, status, amount, fees, timestamp, details FROM payments WHERE id = ?",
        )?;

        let result = stmt.query_row(params![id], |row| {
            Ok(Payment {
                id: row.get(0)?,
                payment_type: row.get::<_, String>(1)?.parse().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        "Failed to parse payment type".into(),
                    )
                })?,
                status: row.get::<_, String>(2)?.parse().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        "Failed to parse payment status".into(),
                    )
                })?,
                amount: row.get(3)?,
                fees: row.get(4)?,
                timestamp: row.get(5)?,
                details: row.get(6)?,
            })
        });
        result.map_err(StorageError::from)
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

#[cfg(test)]
mod tests {
    use crate::{PaymentStatus, PaymentType};

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
        storage.insert_payment(&payment).unwrap();

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
        let retrieved_payment = storage.get_payment_by_id(&payment.id).unwrap();
        assert_eq!(retrieved_payment.id, payment.id);
        assert_eq!(retrieved_payment.payment_type, payment.payment_type);
        assert_eq!(retrieved_payment.status, payment.status);
        assert_eq!(retrieved_payment.amount, payment.amount);
        assert_eq!(retrieved_payment.fees, payment.fees);
    }
}
