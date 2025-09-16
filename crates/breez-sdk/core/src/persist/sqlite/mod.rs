#![allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use macros::async_trait;
use sqlx::{Row, SqlitePool, migrate::MigrateDatabase};
use std::path::Path;

use crate::{
    DepositInfo, PaymentDetails, PaymentMethod,
    models::{PaymentStatus, PaymentType},
    persist::{PaymentMetadata, UpdateDepositPayload},
};

use super::{Payment, Storage, StorageError};

const DEFAULT_DB_FILENAME: &str = "storage.sql";

/// SQLite-based storage implementation
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        StorageError::Implementation(err.to_string())
    }
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
    pub async fn new(path: &Path) -> Result<Self, StorageError> {
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        std::fs::create_dir_all(path)
            .map_err(|e| StorageError::InitializationError(e.to_string()))?;

        let db_path = path.join(DEFAULT_DB_FILENAME);
        // Create an in-memory database for tests to avoid file permission issues
        let db_url = format!("sqlite://{}", db_path.display());

        if !sqlx::Sqlite::database_exists(&db_url).await? {
            sqlx::Sqlite::create_database(&db_url).await?;
        }

        // Connect to the database
        let pool = SqlitePool::connect(&db_url).await?;

        migrate(&pool).await?;
        Ok(Self { pool })
    }
}

async fn migrate(pool: &sqlx::SqlitePool) -> Result<(), StorageError> {
    match sqlx::migrate!("src/persist/sqlite/migrations")
        .run(pool)
        .await
    {
        Ok(()) => Ok(()),
        Err(e) => Err(StorageError::InitializationError(format!(
            "Failed to run migrations: {e}"
        ))),
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, StorageError> {
        let query = format!(
            "SELECT p.id, p.payment_type, p.status, p.amount, p.fees, p.timestamp, p.details, p.method, pm.lnurl_pay_info
             FROM payments p
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             ORDER BY p.timestamp DESC 
             LIMIT {} OFFSET {}",
            limit.unwrap_or(u32::MAX),
            offset.unwrap_or(0)
        );

        let payments = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| {
                let id: String = row.get(0);
                let payment_type_str: String = row.get(1);
                let status_str: String = row.get(2);
                let amount: i64 = row.get(3);
                let fees: i64 = row.get(4);
                let timestamp: i64 = row.get(5);
                let details_json: Option<String> = row.get(6);
                let method_str: Option<String> = row.get(7);
                let lnurl_pay_info_json: Option<String> = row.get(8);

                let mut details: Option<PaymentDetails> = None;
                if let Some(details_str) = details_json {
                    details = Some(serde_json::from_str(&details_str)?);
                }

                // Handle the Lightning case
                if let Some(PaymentDetails::Lightning {
                    ref mut lnurl_pay_info,
                    ..
                }) = details
                    && let Some(lnurl_info_str) = lnurl_pay_info_json
                {
                    *lnurl_pay_info = serde_json::from_str(&lnurl_info_str)?;
                }

                let method: PaymentMethod = if let Some(method_str) = method_str {
                    method_str.parse().map_err(|()| {
                        StorageError::InitializationError(
                            "failed to parse payment method".to_string(),
                        )
                    })?
                } else {
                    // Create a default payment method if none was provided
                    PaymentMethod::Unknown
                };

                Ok(Payment {
                    id,
                    payment_type: PaymentType::from(payment_type_str.as_str()),
                    status: PaymentStatus::from(status_str.as_str()),
                    amount: amount as u64,
                    fees: fees as u64,
                    timestamp: timestamp as u64,
                    details,
                    method,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        Ok(payments)
    }

    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError> {
        let details_json = if let Some(details) = &payment.details {
            Some(serde_json::to_string(details)?)
        } else {
            None
        };

        sqlx::query(
            "INSERT OR REPLACE INTO payments (id, payment_type, status, amount, fees, timestamp, details, method) 
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&payment.id)
            .bind(payment.payment_type.to_string())
            .bind(payment.status.to_string())
            .bind(payment.amount as i64)
            .bind(payment.fees as i64)
            .bind(payment.timestamp as i64)
            .bind(&details_json)
            .bind(payment.method.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        let lnurl_pay_info_json = if let Some(info) = &metadata.lnurl_pay_info {
            Some(serde_json::to_string(info)?)
        } else {
            None
        };

        sqlx::query(
            "INSERT OR REPLACE INTO payment_metadata (payment_id, lnurl_pay_info) VALUES (?, ?)",
        )
        .bind(&payment_id)
        .bind(&lnurl_pay_info_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(&key)
            .bind(&value)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
        let result = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(&key)
            .fetch_optional(&self.pool)
            .await?;

        match result {
            Some(row) => {
                let value: String = row.get(0);
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(&key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError> {
        let query = "SELECT id, payment_type, status, amount, fees, timestamp, details, method, pm.lnurl_pay_info 
                     FROM payments 
                     LEFT JOIN payment_metadata pm ON payments.id = pm.payment_id 
                     WHERE payments.id = ?";

        let row = sqlx::query(query).bind(&id).fetch_one(&self.pool).await?;

        let id: String = row.get(0);
        let payment_type_str: String = row.get(1);
        let status_str: String = row.get(2);
        let amount: i64 = row.get(3);
        let fees: i64 = row.get(4);
        let timestamp: i64 = row.get(5);
        let details_json: Option<String> = row.get(6);
        let method_str: String = row.get(7);
        let lnurl_pay_info_json: Option<String> = row.get(8);

        let mut details: Option<PaymentDetails> = None;
        if let Some(details_str) = &details_json {
            details = Some(serde_json::from_str(details_str)?);
        }

        // Handle the Lightning case
        if let Some(PaymentDetails::Lightning {
            ref mut lnurl_pay_info,
            ..
        }) = details
            && let Some(lnurl_info_str) = &lnurl_pay_info_json
        {
            *lnurl_pay_info = serde_json::from_str(lnurl_info_str)?;
        }

        let method: PaymentMethod = method_str.parse().map_err(|()| {
            StorageError::Implementation("failed to parse payment method".to_string())
        })?;

        Ok(Payment {
            id,
            payment_type: PaymentType::from(payment_type_str.as_str()),
            status: PaymentStatus::from(status_str.as_str()),
            amount: amount as u64,
            fees: fees as u64,
            timestamp: timestamp as u64,
            details,
            method,
        })
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT OR IGNORE INTO unclaimed_deposits (txid, vout, amount_sats) VALUES (?, ?, ?)",
        )
        .bind(&txid)
        .bind(vout)
        .bind(amount_sats as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM unclaimed_deposits WHERE txid = ? AND vout = ?")
            .bind(&txid)
            .bind(vout)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        let deposits = sqlx::query("SELECT txid, vout, amount_sats, claim_error, refund_tx, refund_tx_id FROM unclaimed_deposits")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| {
                let txid: String = row.get(0);
                let vout: u32 = row.get(1);
                let amount_sats: u64 = row.get::<i64, _>(2) as u64;
                let claim_error_json: Option<String> = row.get(3);
                let refund_tx: Option<String> = row.get(4);
                let refund_tx_id: Option<String> = row.get(5);
                let claim_error = match &claim_error_json {
                    Some(error_str) => Some(serde_json::from_str(error_str)?),
                    None => None,
                };

                Ok(DepositInfo {
                    txid,
                    vout,
                    amount_sats,
                    refund_tx,
                    refund_tx_id,
                    claim_error,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        Ok(deposits)
    }

    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError> {
        match payload {
            UpdateDepositPayload::ClaimError { error } => {
                let error = serde_json::to_string(&error)?;
                sqlx::query(
                    "UPDATE unclaimed_deposits SET claim_error = ? WHERE txid = ? AND vout = ?",
                )
                .bind(&error)
                .bind(&txid)
                .bind(vout)
                .execute(&self.pool)
                .await?;
            }
            UpdateDepositPayload::Refund {
                refund_txid,
                refund_tx,
            } => {
                sqlx::query("UPDATE unclaimed_deposits SET refund_tx = ?, refund_tx_id = ? WHERE txid = ? AND vout = ?")
                    .bind(&refund_tx)
                    .bind(&refund_txid)
                    .bind(&txid)
                    .bind(vout)
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::SqliteStorage;

    #[tokio::test]
    async fn test_sqlite_storage() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).await.unwrap();

        crate::persist::tests::test_sqlite_storage(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_unclaimed_deposits_crud() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_deposits").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).await.unwrap();

        crate::persist::tests::test_unclaimed_deposits_crud(Box::new(storage)).await;
    }

    #[tokio::test]
    async fn test_deposit_refunds() {
        let temp_dir = tempdir::TempDir::new("sqlite_storage_refund_tx").unwrap();
        let storage = SqliteStorage::new(temp_dir.path()).await.unwrap();

        crate::persist::tests::test_deposit_refunds(Box::new(storage)).await;
    }
}
