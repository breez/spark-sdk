#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub(crate) mod sqlite;

use std::sync::Arc;

use macros::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    DepositClaimError, DepositInfo, GetLightningAddressResponse, LnurlPayInfo,
    SetLightningAddressRequest, models::Payment,
};

const ACCOUNT_INFO_KEY: &str = "account_info";
const LIGHTNING_ADDRESS_KEY: &str = "lightning_address";
const LIGHTNING_ADDRESS_CONFIG_KEY: &str = "lightning_address_config";
const SYNC_OFFSET_KEY: &str = "sync_offset";
const TX_CACHE_KEY: &str = "tx_cache";
const STATIC_DEPOSIT_ADDRESS_CACHE_KEY: &str = "static_deposit_address";

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum UpdateDepositPayload {
    ClaimError {
        error: DepositClaimError,
    },
    Refund {
        refund_txid: String,
        refund_tx: String,
    },
}

/// Errors that can occur during storage operations
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum StorageError {
    #[error("Underline implementation error: {0}")]
    Implementation(String),

    /// Database initialization error
    #[error("Failed to initialize database: {0}")]
    InitializationError(String),

    #[error("Failed to serialize/deserialize data: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}

/// Metadata associated with a payment that cannot be extracted from the Spark operator.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentMetadata {
    pub lnurl_pay_info: Option<LnurlPayInfo>,
}

/// Trait for persistent storage
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait]
pub trait Storage: Send + Sync {
    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError>;
    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError>;
    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError>;
    /// Lists payments with pagination
    ///
    /// # Arguments
    ///
    /// * `offset` - Number of records to skip
    /// * `limit` - Maximum number of records to return
    ///
    /// # Returns
    ///
    /// A vector of payments or a `StorageError`
    async fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, StorageError>;

    /// Inserts a payment into storage
    ///
    /// # Arguments
    ///
    /// * `payment` - The payment to insert
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError>;

    /// Inserts payment metadata into storage
    ///
    /// # Arguments
    ///
    /// * `payment_id` - The ID of the payment
    /// * `metadata` - The metadata to insert
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError>;

    /// Gets a payment by its ID
    /// # Arguments
    ///
    /// * `id` - The ID of the payment to retrieve
    ///
    /// # Returns
    ///
    /// The payment if found or None if not found
    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError>;

    /// Add a deposit to storage
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    /// * `amount_sats` - The amount of the deposit in sats
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError>;

    /// Removes an unclaimed deposit from storage
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError>;

    /// Lists all unclaimed deposits from storage
    /// # Returns
    ///
    /// A vector of `DepositInfo` or a `StorageError`
    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError>;

    /// Updates or inserts unclaimed deposit details
    /// # Arguments
    ///
    /// * `txid` - The transaction ID of the deposit
    /// * `vout` - The output index of the deposit
    /// * `payload` - The payload for the update
    ///
    /// # Returns
    ///
    /// Success or a `StorageError`
    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError>;
}

pub(crate) struct ObjectCacheRepository {
    storage: Arc<dyn Storage>,
}

impl ObjectCacheRepository {
    pub(crate) fn new(storage: Arc<dyn Storage>) -> Self {
        ObjectCacheRepository { storage }
    }

    pub(crate) async fn save_account_info(
        &self,
        value: &CachedAccountInfo,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(ACCOUNT_INFO_KEY.to_string(), serde_json::to_string(value)?)
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_account_info(
        &self,
    ) -> Result<Option<CachedAccountInfo>, StorageError> {
        let value = self
            .storage
            .get_cached_item(ACCOUNT_INFO_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_sync_info(&self, value: &CachedSyncInfo) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(SYNC_OFFSET_KEY.to_string(), serde_json::to_string(value)?)
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_sync_info(&self) -> Result<Option<CachedSyncInfo>, StorageError> {
        let value = self
            .storage
            .get_cached_item(SYNC_OFFSET_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_tx(&self, txid: &str, value: &CachedTx) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                format!("{TX_CACHE_KEY}-{txid}"),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_tx(&self, txid: &str) -> Result<Option<CachedTx>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!("{TX_CACHE_KEY}-{txid}"))
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_static_deposit_address(
        &self,
        value: &StaticDepositAddress,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                STATIC_DEPOSIT_ADDRESS_CACHE_KEY.to_string(),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_static_deposit_address(
        &self,
    ) -> Result<Option<StaticDepositAddress>, StorageError> {
        let value = self
            .storage
            .get_cached_item(STATIC_DEPOSIT_ADDRESS_CACHE_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_lightning_address_config(
        &self,
        value: &SetLightningAddressRequest,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                LIGHTNING_ADDRESS_CONFIG_KEY.to_string(),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn delete_lightning_address_config(&self) -> Result<(), StorageError> {
        self.storage
            .delete_cached_item(LIGHTNING_ADDRESS_CONFIG_KEY.to_string())
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_lightning_address_config(
        &self,
    ) -> Result<Option<SetLightningAddressRequest>, StorageError> {
        let value = self
            .storage
            .get_cached_item(LIGHTNING_ADDRESS_CONFIG_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_lightning_address(
        &self,
        value: &GetLightningAddressResponse,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                LIGHTNING_ADDRESS_KEY.to_string(),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn delete_lightning_address(&self) -> Result<(), StorageError> {
        self.storage
            .delete_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_lightning_address(
        &self,
    ) -> Result<Option<GetLightningAddressResponse>, StorageError> {
        let value = self
            .storage
            .get_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedAccountInfo {
    pub(crate) balance_sats: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedSyncInfo {
    pub(crate) offset: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedTx {
    pub(crate) raw_tx: String,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct StaticDepositAddress {
    pub(crate) address: String,
}

#[cfg(feature = "test-utils")]
pub mod tests {
    use crate::{
        DepositClaimError, Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType,
        Storage, UpdateDepositPayload,
    };
    use chrono::Utc;

    pub async fn test_sqlite_storage(storage: Box<dyn Storage>) {
        // Create test payment
        let payment = Payment {
            id: "pmt123".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 100_000,
            fees: 1000,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark),
        };

        // Insert payment
        storage.insert_payment(payment.clone()).await.unwrap();

        // List payments
        let payments = storage.list_payments(Some(0), Some(10)).await.unwrap();
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].id, payment.id);
        assert_eq!(payments[0].payment_type, payment.payment_type);
        assert_eq!(payments[0].status, payment.status);
        assert_eq!(payments[0].amount, payment.amount);
        assert_eq!(payments[0].fees, payment.fees);
        assert!(matches!(payments[0].details, Some(PaymentDetails::Spark)));

        // Get payment by ID
        let retrieved_payment = storage.get_payment_by_id(payment.id.clone()).await.unwrap();
        assert_eq!(retrieved_payment.id, payment.id);
        assert_eq!(retrieved_payment.payment_type, payment.payment_type);
        assert_eq!(retrieved_payment.status, payment.status);
        assert_eq!(retrieved_payment.amount, payment.amount);
        assert_eq!(retrieved_payment.fees, payment.fees);
        assert!(matches!(
            retrieved_payment.details,
            Some(PaymentDetails::Spark)
        ));
    }

    pub async fn test_unclaimed_deposits_crud(storage: Box<dyn Storage>) {
        // Initially, list should be empty
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 0);

        // Add first deposit
        storage
            .add_deposit("tx123".to_string(), 0, 50000)
            .await
            .unwrap();
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "tx123");
        assert_eq!(deposits[0].vout, 0);
        assert_eq!(deposits[0].amount_sats, 50000);
        assert!(deposits[0].claim_error.is_none());

        // Add second deposit
        storage
            .add_deposit("tx456".to_string(), 1, 75000)
            .await
            .unwrap();
        storage
            .update_deposit(
                "tx456".to_string(),
                1,
                UpdateDepositPayload::ClaimError {
                    error: DepositClaimError::Generic {
                        message: "Test error".to_string(),
                    },
                },
            )
            .await
            .unwrap();
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 2);

        // Find deposit2 in the list
        let deposit2_found = deposits.iter().find(|d| d.txid == "tx456").unwrap();
        assert_eq!(deposit2_found.vout, 1);
        assert_eq!(deposit2_found.amount_sats, 75000);
        assert!(deposit2_found.claim_error.is_some());

        // Remove first deposit
        storage
            .delete_deposit("tx123".to_string(), 0)
            .await
            .unwrap();
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "tx456");

        // Remove second deposit
        storage
            .delete_deposit("tx456".to_string(), 1)
            .await
            .unwrap();
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 0);
    }

    pub async fn test_deposit_refunds(storage: Box<dyn Storage>) {
        // Add the initial deposit
        storage
            .add_deposit("test_tx_123".to_string(), 0, 100_000)
            .await
            .unwrap();
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "test_tx_123");
        assert_eq!(deposits[0].vout, 0);
        assert_eq!(deposits[0].amount_sats, 100_000);
        assert!(deposits[0].claim_error.is_none());

        // Update the deposit refund information
        storage
            .update_deposit(
                "test_tx_123".to_string(),
                0,
                UpdateDepositPayload::Refund {
                    refund_txid: "refund_tx_id_456".to_string(),
                    refund_tx: "0200000001abcd1234...".to_string(),
                },
            )
            .await
            .unwrap();

        // Verify that the deposit information remains unchanged
        let deposits = storage.list_deposits().await.unwrap();
        assert_eq!(deposits.len(), 1);
        assert_eq!(deposits[0].txid, "test_tx_123");
        assert_eq!(deposits[0].vout, 0);
        assert_eq!(deposits[0].amount_sats, 100_000);
        assert!(deposits[0].claim_error.is_none());
        assert_eq!(
            deposits[0].refund_tx_id,
            Some("refund_tx_id_456".to_string())
        );
        assert_eq!(
            deposits[0].refund_tx,
            Some("0200000001abcd1234...".to_string())
        );
    }
}
