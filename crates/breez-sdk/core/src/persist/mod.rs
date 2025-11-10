pub(crate) mod path;
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub(crate) mod sqlite;

use std::{collections::HashMap, sync::Arc};

use breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails;
use macros::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    DepositClaimError, DepositInfo, LightningAddressInfo, ListPaymentsRequest, LnurlPayInfo,
    LnurlWithdrawInfo, TokenBalance, TokenMetadata, models::Payment,
};

const ACCOUNT_INFO_KEY: &str = "account_info";
const LIGHTNING_ADDRESS_KEY: &str = "lightning_address";
const SYNC_OFFSET_KEY: &str = "sync_offset";
const TX_CACHE_KEY: &str = "tx_cache";
const STATIC_DEPOSIT_ADDRESS_CACHE_KEY: &str = "static_deposit_address";
const TOKEN_METADATA_KEY_PREFIX: &str = "token_metadata_";
const PAYMENT_REQUEST_METADATA_KEY_PREFIX: &str = "payment_request_metadata";
const SPARK_PRIVATE_MODE_INITIALIZED_KEY: &str = "spark_private_mode_initialized";

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
#[derive(Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentMetadata {
    pub lnurl_pay_info: Option<LnurlPayInfo>,
    pub lnurl_withdraw_info: Option<LnurlWithdrawInfo>,
    pub lnurl_description: Option<String>,
}

/// Trait for persistent storage
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait]
pub trait Storage: Send + Sync {
    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError>;
    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError>;
    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError>;
    /// Lists payments with optional filters and pagination
    ///
    /// # Arguments
    ///
    /// * `list_payments_request` - The request to list payments
    ///
    /// # Returns
    ///
    /// A vector of payments or a `StorageError`
    async fn list_payments(
        &self,
        request: ListPaymentsRequest,
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

    /// Gets a payment by its invoice
    /// # Arguments
    ///
    /// * `invoice` - The invoice of the payment to retrieve
    /// # Returns
    ///
    /// The payment if found or None if not found
    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError>;

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

    pub(crate) async fn save_lightning_address(
        &self,
        value: &LightningAddressInfo,
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
    ) -> Result<Option<LightningAddressInfo>, StorageError> {
        let value = self
            .storage
            .get_cached_item(LIGHTNING_ADDRESS_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_token_metadata(
        &self,
        value: &TokenMetadata,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                format!("{TOKEN_METADATA_KEY_PREFIX}{}", value.identifier),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_token_metadata(
        &self,
        identifier: &str,
    ) -> Result<Option<TokenMetadata>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!("{TOKEN_METADATA_KEY_PREFIX}{identifier}"))
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_payment_request_metadata(
        &self,
        value: &PaymentRequestMetadata,
    ) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                format!(
                    "{PAYMENT_REQUEST_METADATA_KEY_PREFIX}-{}",
                    value.payment_request
                ),
                serde_json::to_string(value)?,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_payment_request_metadata(
        &self,
        payment_request: &str,
    ) -> Result<Option<PaymentRequestMetadata>, StorageError> {
        let value = self
            .storage
            .get_cached_item(format!(
                "{PAYMENT_REQUEST_METADATA_KEY_PREFIX}-{payment_request}",
            ))
            .await?;
        match value {
            Some(value) => Ok(Some(serde_json::from_str(&value)?)),
            None => Ok(None),
        }
    }

    pub(crate) async fn delete_payment_request_metadata(
        &self,
        payment_request: &str,
    ) -> Result<(), StorageError> {
        self.storage
            .delete_cached_item(format!(
                "{PAYMENT_REQUEST_METADATA_KEY_PREFIX}-{payment_request}",
            ))
            .await?;
        Ok(())
    }

    pub(crate) async fn save_spark_private_mode_initialized(&self) -> Result<(), StorageError> {
        self.storage
            .set_cached_item(
                SPARK_PRIVATE_MODE_INITIALIZED_KEY.to_string(),
                "true".to_string(),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn fetch_spark_private_mode_initialized(&self) -> Result<bool, StorageError> {
        let value = self
            .storage
            .get_cached_item(SPARK_PRIVATE_MODE_INITIALIZED_KEY.to_string())
            .await?;
        match value {
            Some(value) => Ok(value == "true"),
            None => Ok(false),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedAccountInfo {
    pub(crate) balance_sats: u64,
    #[serde(default)]
    pub(crate) token_balances: HashMap<String, TokenBalance>,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedSyncInfo {
    pub(crate) offset: u64,
    pub(crate) last_synced_final_token_payment_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CachedTx {
    pub(crate) raw_tx: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct PaymentRequestMetadata {
    pub payment_request: String,
    pub lnurl_withdraw_request_details: LnurlWithdrawRequestDetails,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct StaticDepositAddress {
    pub(crate) address: String,
}

#[cfg(feature = "test-utils")]
pub mod tests {
    use breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails;

    use chrono::Utc;

    use crate::{
        DepositClaimError, ListPaymentsRequest, LnurlWithdrawInfo, Payment, PaymentDetails,
        PaymentMetadata, PaymentMethod, PaymentStatus, PaymentType, Storage, UpdateDepositPayload,
        persist::{ObjectCacheRepository, PaymentRequestMetadata},
    };

    #[allow(clippy::too_many_lines)]
    pub async fn test_sqlite_sync_storage(
        storage: Box<dyn breez_sdk_common::sync::storage::SyncStorage>,
    ) {
        use breez_sdk_common::sync::RecordId;
        use breez_sdk_common::sync::storage::{Record, UnversionedRecordChange};
        use std::collections::HashMap;

        // Test 1: Initial state - get_last_revision should return 0
        let last_revision = storage.get_last_revision().await.unwrap();
        assert_eq!(last_revision, 0, "Initial last revision should be 0");

        // Test 2: No pending outgoing changes initially
        let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
        assert_eq!(pending.len(), 0, "Should have no pending outgoing changes");

        // Test 3: No incoming records initially
        let incoming = storage.get_incoming_records(10).await.unwrap();
        assert_eq!(incoming.len(), 0, "Should have no incoming records");

        // Test 4: No latest outgoing change initially
        let latest = storage.get_latest_outgoing_change().await.unwrap();
        assert!(latest.is_none(), "Should have no latest outgoing change");

        // Test 5: Add outgoing change (create new record)
        let mut updated_fields = HashMap::new();
        updated_fields.insert("name".to_string(), "\"Alice\"".to_string());
        updated_fields.insert("age".to_string(), "30".to_string());

        let change1 = UnversionedRecordChange {
            id: RecordId::new("user".to_string(), "user1".to_string()),
            schema_version: "1.0.0".to_string(),
            updated_fields: updated_fields.clone(),
        };

        let revision1 = storage.add_outgoing_change(change1).await.unwrap();
        assert!(revision1 > 0, "First revision should be greater than 0");

        // Test 6: Check pending outgoing changes
        let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
        assert_eq!(pending.len(), 1, "Should have 1 pending outgoing change");
        assert_eq!(pending[0].change.id.r#type, "user");
        assert_eq!(pending[0].change.id.data_id, "user1");
        assert_eq!(pending[0].change.revision, revision1);
        assert_eq!(pending[0].change.schema_version, "1.0.0");
        assert!(
            pending[0].parent.is_none(),
            "First change should have no parent"
        );

        // Test 7: Get latest outgoing change
        let latest = storage.get_latest_outgoing_change().await.unwrap();
        assert!(latest.is_some());
        let latest = latest.unwrap();
        assert_eq!(latest.change.id.r#type, "user");
        assert_eq!(latest.change.revision, revision1);

        // Test 8: Complete outgoing sync (moves to sync_state)
        let mut complete_data = HashMap::new();
        complete_data.insert("name".to_string(), "\"Alice\"".to_string());
        complete_data.insert("age".to_string(), "30".to_string());

        let completed_record = Record {
            id: RecordId::new("user".to_string(), "user1".to_string()),
            revision: revision1,
            schema_version: "1.0.0".to_string(),
            data: complete_data,
        };

        storage
            .complete_outgoing_sync(completed_record.clone())
            .await
            .unwrap();

        // Test 9: Pending changes should now be empty
        let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
        assert_eq!(
            pending.len(),
            0,
            "Should have no pending changes after completion"
        );

        // Test 10: Last revision should be updated
        let last_revision = storage.get_last_revision().await.unwrap();
        assert_eq!(
            last_revision, revision1,
            "Last revision should match completed revision"
        );

        // Test 11: Add another outgoing change (update existing record)
        let mut updated_fields2 = HashMap::new();
        updated_fields2.insert("age".to_string(), "31".to_string());

        let change2 = UnversionedRecordChange {
            id: RecordId::new("user".to_string(), "user1".to_string()),
            schema_version: "1.0.0".to_string(),
            updated_fields: updated_fields2,
        };

        let revision2 = storage.add_outgoing_change(change2).await.unwrap();
        assert!(
            revision2 > revision1,
            "Second revision should be greater than first"
        );

        // Test 12: Check pending changes now includes parent
        let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
        assert_eq!(pending.len(), 1, "Should have 1 pending change");
        assert!(
            pending[0].parent.is_some(),
            "Update should have parent record"
        );
        let parent = pending[0].parent.as_ref().unwrap();
        assert_eq!(parent.revision, revision1);
        assert_eq!(parent.id.r#type, "user");

        // Test 13: Insert incoming records
        let mut incoming_data1 = HashMap::new();
        incoming_data1.insert("title".to_string(), "\"Post 1\"".to_string());
        incoming_data1.insert("content".to_string(), "\"Hello World\"".to_string());

        let incoming_record1 = Record {
            id: RecordId::new("post".to_string(), "post1".to_string()),
            revision: 100,
            schema_version: "1.0.0".to_string(),
            data: incoming_data1,
        };

        let mut incoming_data2 = HashMap::new();
        incoming_data2.insert("title".to_string(), "\"Post 2\"".to_string());

        let incoming_record2 = Record {
            id: RecordId::new("post".to_string(), "post2".to_string()),
            revision: 101,
            schema_version: "1.0.0".to_string(),
            data: incoming_data2,
        };

        storage
            .insert_incoming_records(vec![incoming_record1.clone(), incoming_record2.clone()])
            .await
            .unwrap();

        // Test 14: Get incoming records
        let incoming = storage.get_incoming_records(10).await.unwrap();
        assert_eq!(incoming.len(), 2, "Should have 2 incoming records");
        assert_eq!(incoming[0].new_state.id.r#type, "post");
        assert_eq!(incoming[0].new_state.revision, 100);
        assert!(
            incoming[0].old_state.is_none(),
            "New incoming record should have no old state"
        );

        // Test 15: Update record from incoming (moves to sync_state)
        storage
            .update_record_from_incoming(incoming_record1.clone())
            .await
            .unwrap();

        // Test 16: Delete incoming record
        storage
            .delete_incoming_record(incoming_record1.clone())
            .await
            .unwrap();

        // Test 17: Check incoming records after deletion
        let incoming = storage.get_incoming_records(10).await.unwrap();
        assert_eq!(incoming.len(), 1, "Should have 1 incoming record remaining");
        assert_eq!(incoming[0].new_state.id.data_id, "post2");

        // Test 18: Insert incoming record that updates existing state
        let mut updated_incoming_data = HashMap::new();
        updated_incoming_data.insert("title".to_string(), "\"Post 1 Updated\"".to_string());
        updated_incoming_data.insert("content".to_string(), "\"Updated content\"".to_string());

        let updated_incoming_record = Record {
            id: RecordId::new("post".to_string(), "post1".to_string()),
            revision: 102,
            schema_version: "1.0.0".to_string(),
            data: updated_incoming_data,
        };

        storage
            .insert_incoming_records(vec![updated_incoming_record.clone()])
            .await
            .unwrap();

        // Test 19: Get incoming records with old_state
        let incoming = storage.get_incoming_records(10).await.unwrap();
        let post1_update = incoming.iter().find(|r| r.new_state.id.data_id == "post1");
        assert!(post1_update.is_some(), "Should find post1 update");
        let post1_update = post1_update.unwrap();
        assert!(
            post1_update.old_state.is_some(),
            "Update should have old state"
        );
        assert_eq!(
            post1_update.old_state.as_ref().unwrap().revision,
            100,
            "Old state should be original revision"
        );

        // Test 20: Rebase pending outgoing records
        storage.rebase_pending_outgoing_records(150).await.unwrap();

        // Test 21: Check that pending outgoing change revision was updated
        let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
        assert!(
            pending[0].change.revision > revision2,
            "Revision should be rebased"
        );

        // Test 22: Test limit on pending outgoing changes
        // Add multiple changes
        for i in 0..5 {
            let mut fields = HashMap::new();
            fields.insert("value".to_string(), format!("\"{i}\""));

            let change = UnversionedRecordChange {
                id: RecordId::new("test".to_string(), format!("test{i}")),
                schema_version: "1.0.0".to_string(),
                updated_fields: fields,
            };
            storage.add_outgoing_change(change).await.unwrap();
        }

        let pending_limited = storage.get_pending_outgoing_changes(3).await.unwrap();
        assert_eq!(
            pending_limited.len(),
            3,
            "Should respect limit on pending changes"
        );

        // Test 23: Test limit on incoming records
        let incoming_limited = storage.get_incoming_records(1).await.unwrap();
        assert_eq!(
            incoming_limited.len(),
            1,
            "Should respect limit on incoming records"
        );

        // Test 24: Test ordering - pending outgoing should be ordered by revision ASC
        let all_pending = storage.get_pending_outgoing_changes(100).await.unwrap();
        for i in 1..all_pending.len() {
            assert!(
                all_pending[i].change.revision >= all_pending[i.saturating_sub(1)].change.revision,
                "Pending changes should be ordered by revision ascending"
            );
        }

        // Test 25: Test ordering - incoming should be ordered by revision ASC
        let all_incoming = storage.get_incoming_records(100).await.unwrap();
        for i in 1..all_incoming.len() {
            assert!(
                all_incoming[i].new_state.revision
                    >= all_incoming[i.saturating_sub(1)].new_state.revision,
                "Incoming records should be ordered by revision ascending"
            );
        }

        // Test 26: Test empty insert_incoming_records
        storage.insert_incoming_records(vec![]).await.unwrap();

        // Test 27: Test different record types
        let mut settings_fields = HashMap::new();
        settings_fields.insert("theme".to_string(), "\"dark\"".to_string());

        let settings_change = UnversionedRecordChange {
            id: RecordId::new("settings".to_string(), "global".to_string()),
            schema_version: "2.0.0".to_string(),
            updated_fields: settings_fields,
        };

        let settings_revision = storage.add_outgoing_change(settings_change).await.unwrap();

        let pending = storage.get_pending_outgoing_changes(100).await.unwrap();
        let settings_pending = pending.iter().find(|p| p.change.id.r#type == "settings");
        assert!(settings_pending.is_some(), "Should find settings change");
        assert_eq!(
            settings_pending.unwrap().change.schema_version,
            "2.0.0",
            "Should preserve schema version"
        );

        // Test 28: Complete multiple types
        let mut complete_settings_data = HashMap::new();
        complete_settings_data.insert("theme".to_string(), "\"dark\"".to_string());

        let completed_settings = Record {
            id: RecordId::new("settings".to_string(), "global".to_string()),
            revision: settings_revision,
            schema_version: "2.0.0".to_string(),
            data: complete_settings_data,
        };

        storage
            .complete_outgoing_sync(completed_settings)
            .await
            .unwrap();

        let last_revision = storage.get_last_revision().await.unwrap();
        assert!(
            last_revision >= settings_revision,
            "Last revision should be at least settings revision"
        );
    }

    #[allow(clippy::too_many_lines)]
    pub async fn test_sqlite_storage(storage: Box<dyn Storage>) {
        use crate::models::{LnurlPayInfo, TokenMetadata};

        // Test 1: Spark payment
        let spark_payment = Payment {
            id: "spark_pmt123".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: u128::from(u64::MAX).checked_add(100_000).unwrap(),
            fees: 1000,
            timestamp: 5000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: Some(crate::SparkInvoicePaymentDetails {
                    description: Some("description".to_string()),
                    invoice: "invoice_string".to_string(),
                }),
            }),
        };

        // Test 2: Token payment
        let token_metadata = TokenMetadata {
            identifier: "token123".to_string(),
            issuer_public_key:
                "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab".to_string(),
            name: "Test Token".to_string(),
            ticker: "TTK".to_string(),
            decimals: 8,
            max_supply: 21_000_000,
            is_freezable: false,
        };
        let token_payment = Payment {
            id: "token_pmt456".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Pending,
            amount: 50_000,
            fees: 500,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: token_metadata.clone(),
                tx_hash: "tx_hash".to_string(),
                invoice_details: Some(crate::SparkInvoicePaymentDetails {
                    description: Some("description_2".to_string()),
                    invoice: "invoice_string_2".to_string(),
                }),
            }),
        };

        // Test 3: Lightning payment with full details
        let pay_metadata = PaymentMetadata {
            lnurl_pay_info: Some(LnurlPayInfo {
                ln_address: Some("test@example.com".to_string()),
                comment: Some("Test comment".to_string()),
                domain: Some("example.com".to_string()),
                metadata: Some("[[\"text/plain\", \"Test metadata\"]]".to_string()),
                processed_success_action: None,
                raw_success_action: None,
            }),
            lnurl_withdraw_info: None,
            lnurl_description: None,
        };
        let lightning_lnurl_pay_payment = Payment {
            id: "lightning_pmt789".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 25_000,
            fees: 250,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: Some("Test lightning payment".to_string()),
                preimage: Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab".to_string()),
                invoice: "lnbc250n1pjqxyz9pp5abc123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
                payment_hash: "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321".to_string(),
                destination_pubkey: "03123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef01".to_string(),
                lnurl_pay_info: pay_metadata.lnurl_pay_info.clone(),
                lnurl_withdraw_info: pay_metadata.lnurl_withdraw_info.clone(),
            }),
        };

        // Test 4: Lightning payment with full details
        let withdraw_metadata = PaymentMetadata {
            lnurl_pay_info: None,
            lnurl_withdraw_info: Some(LnurlWithdrawInfo {
                withdraw_url: "http://example.com/withdraw".to_string(),
            }),
            lnurl_description: None,
        };
        let lightning_lnurl_withdraw_payment = Payment {
            id: "lightning_pmtabc".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 75_000,
            fees: 750,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: Some("Test lightning payment".to_string()),
                preimage: Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab".to_string()),
                invoice: "lnbc250n1pjqxyz9pp5abc123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
                payment_hash: "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321".to_string(),
                destination_pubkey: "03123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef01".to_string(),
                lnurl_pay_info: withdraw_metadata.lnurl_pay_info.clone(),
                lnurl_withdraw_info: withdraw_metadata.lnurl_withdraw_info.clone(),
            }),
        };

        // Test 5: Lightning payment with minimal details
        let lightning_minimal_payment = Payment {
            id: "lightning_minimal_pmt012".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Failed,
            amount: 10_000,
            fees: 100,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: None,
                preimage: None,
                invoice: "lnbc100n1pjqxyz9pp5def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
                payment_hash: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
                destination_pubkey: "02987654321fedcba0987654321fedcba0987654321fedcba0987654321fedcba09".to_string(),
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
            }),
        };

        // Test 6: Withdraw payment
        let withdraw_payment = Payment {
            id: "withdraw_pmt345".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 200_000,
            fees: 2000,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Withdraw,
            details: Some(PaymentDetails::Withdraw {
                tx_id: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef12"
                    .to_string(),
            }),
        };

        // Test 7: Deposit payment
        let deposit_payment = Payment {
            id: "deposit_pmt678".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 150_000,
            fees: 1500,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Deposit,
            details: Some(PaymentDetails::Deposit {
                tx_id: "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321fe"
                    .to_string(),
            }),
        };

        // Test 8: Payment with no details
        let no_details_payment = Payment {
            id: "no_details_pmt901".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Pending,
            amount: 75_000,
            fees: 750,
            timestamp: Utc::now().timestamp().try_into().unwrap(),
            method: PaymentMethod::Unknown,
            details: None,
        };

        let test_payments = vec![
            spark_payment.clone(),
            token_payment.clone(),
            lightning_lnurl_pay_payment.clone(),
            lightning_lnurl_withdraw_payment.clone(),
            lightning_minimal_payment.clone(),
            withdraw_payment.clone(),
            deposit_payment.clone(),
            no_details_payment.clone(),
        ];

        // Insert all payments
        for payment in &test_payments {
            storage.insert_payment(payment.clone()).await.unwrap();
        }
        storage
            .set_payment_metadata(lightning_lnurl_pay_payment.id.clone(), pay_metadata)
            .await
            .unwrap();
        storage
            .set_payment_metadata(
                lightning_lnurl_withdraw_payment.id.clone(),
                withdraw_metadata,
            )
            .await
            .unwrap();

        // List all payments
        let payments = storage
            .list_payments(ListPaymentsRequest {
                offset: Some(0),
                limit: Some(10),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(payments.len(), 8);

        // Test each payment type individually
        for (i, expected_payment) in test_payments.iter().enumerate() {
            let retrieved_payment = storage
                .get_payment_by_id(expected_payment.id.clone())
                .await
                .unwrap();

            // Basic fields
            assert_eq!(retrieved_payment.id, expected_payment.id);
            assert_eq!(
                retrieved_payment.payment_type,
                expected_payment.payment_type
            );
            assert_eq!(retrieved_payment.status, expected_payment.status);
            assert_eq!(retrieved_payment.amount, expected_payment.amount);
            assert_eq!(retrieved_payment.fees, expected_payment.fees);
            assert_eq!(retrieved_payment.method, expected_payment.method);

            // Test payment details persistence
            match (&retrieved_payment.details, &expected_payment.details) {
                (None, None) => {}
                (
                    Some(PaymentDetails::Spark {
                        invoice_details: r_invoice,
                    }),
                    Some(PaymentDetails::Spark {
                        invoice_details: e_invoice,
                    }),
                ) => {
                    assert_eq!(r_invoice, e_invoice);
                }
                (
                    Some(PaymentDetails::Token {
                        metadata: r_metadata,
                        tx_hash: r_tx_hash,
                        invoice_details: r_invoice,
                    }),
                    Some(PaymentDetails::Token {
                        metadata: e_metadata,
                        tx_hash: e_tx_hash,
                        invoice_details: e_invoice,
                    }),
                ) => {
                    assert_eq!(r_metadata.identifier, e_metadata.identifier);
                    assert_eq!(r_metadata.issuer_public_key, e_metadata.issuer_public_key);
                    assert_eq!(r_metadata.name, e_metadata.name);
                    assert_eq!(r_metadata.ticker, e_metadata.ticker);
                    assert_eq!(r_metadata.decimals, e_metadata.decimals);
                    assert_eq!(r_metadata.max_supply, e_metadata.max_supply);
                    assert_eq!(r_metadata.is_freezable, e_metadata.is_freezable);
                    assert_eq!(r_tx_hash, e_tx_hash);
                    assert_eq!(r_invoice, e_invoice);
                }
                (
                    Some(PaymentDetails::Lightning {
                        description: r_description,
                        preimage: r_preimage,
                        invoice: r_invoice,
                        payment_hash: r_hash,
                        destination_pubkey: r_dest_pubkey,
                        lnurl_pay_info: r_pay_lnurl,
                        lnurl_withdraw_info: r_withdraw_lnurl,
                    }),
                    Some(PaymentDetails::Lightning {
                        description: e_description,
                        preimage: e_preimage,
                        invoice: e_invoice,
                        payment_hash: e_hash,
                        destination_pubkey: e_dest_pubkey,
                        lnurl_pay_info: e_pay_lnurl,
                        lnurl_withdraw_info: e_withdraw_lnurl,
                    }),
                ) => {
                    assert_eq!(r_description, e_description);
                    assert_eq!(r_preimage, e_preimage);
                    assert_eq!(r_invoice, e_invoice);
                    assert_eq!(r_hash, e_hash);
                    assert_eq!(r_dest_pubkey, e_dest_pubkey);

                    // Test LNURL pay info if present
                    match (r_pay_lnurl, e_pay_lnurl) {
                        (Some(r_info), Some(e_info)) => {
                            assert_eq!(r_info.ln_address, e_info.ln_address);
                            assert_eq!(r_info.comment, e_info.comment);
                            assert_eq!(r_info.domain, e_info.domain);
                            assert_eq!(r_info.metadata, e_info.metadata);
                        }
                        (None, None) => {}
                        _ => panic!(
                            "LNURL pay info mismatch for payment {}",
                            expected_payment.id
                        ),
                    }

                    // Test LNURL withdraw info if present
                    match (r_withdraw_lnurl, e_withdraw_lnurl) {
                        (Some(r_info), Some(e_info)) => {
                            assert_eq!(r_info.withdraw_url, e_info.withdraw_url);
                        }
                        (None, None) => {}
                        _ => panic!(
                            "LNURL withdraw info mismatch for payment {}",
                            expected_payment.id
                        ),
                    }
                }
                (
                    Some(PaymentDetails::Withdraw { tx_id: r_tx_id }),
                    Some(PaymentDetails::Withdraw { tx_id: e_tx_id }),
                )
                | (
                    Some(PaymentDetails::Deposit { tx_id: r_tx_id }),
                    Some(PaymentDetails::Deposit { tx_id: e_tx_id }),
                ) => {
                    assert_eq!(r_tx_id, e_tx_id);
                }
                _ => panic!(
                    "Payment details mismatch for payment {} (index {})",
                    expected_payment.id, i
                ),
            }
        }

        // Test filtering by payment type
        let send_payments = payments
            .iter()
            .filter(|p| p.payment_type == PaymentType::Send)
            .count();
        let receive_payments = payments
            .iter()
            .filter(|p| p.payment_type == PaymentType::Receive)
            .count();
        assert_eq!(send_payments, 4); // spark, lightning_lnurl_pay, withdraw, no_details
        assert_eq!(receive_payments, 4); // token, lightning_lnurl_withdraw, lightning_minimal, deposit

        // Test filtering by status
        let completed_payments = payments
            .iter()
            .filter(|p| p.status == PaymentStatus::Completed)
            .count();
        let pending_payments = payments
            .iter()
            .filter(|p| p.status == PaymentStatus::Pending)
            .count();
        let failed_payments = payments
            .iter()
            .filter(|p| p.status == PaymentStatus::Failed)
            .count();
        assert_eq!(completed_payments, 5); // spark, lightning_lnurl_pay, lightning_lnurl_withdraw, withdraw, deposit
        assert_eq!(pending_payments, 2); // token, no_details
        assert_eq!(failed_payments, 1); // lightning_minimal

        // Test filtering by method
        let lightning_count = payments
            .iter()
            .filter(|p| p.method == PaymentMethod::Lightning)
            .count();
        assert_eq!(lightning_count, 3); // lightning_lnurl_pay, lightning_lnurl_withdraw and lightning_minimal
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

    pub async fn test_payment_type_filtering(storage: Box<dyn Storage>) {
        // Create test payments with different types
        let send_payment = Payment {
            id: "send_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 10_000,
            fees: 100,
            timestamp: 1000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                invoice: "lnbc1".to_string(),
                payment_hash: "hash1".to_string(),
                destination_pubkey: "pubkey1".to_string(),
                description: None,
                preimage: None,
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
            }),
        };

        let receive_payment = Payment {
            id: "receive_1".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 20_000,
            fees: 200,
            timestamp: 2000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                invoice: "lnbc2".to_string(),
                payment_hash: "hash2".to_string(),
                destination_pubkey: "pubkey2".to_string(),
                description: None,
                preimage: None,
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
            }),
        };

        storage.insert_payment(send_payment).await.unwrap();
        storage.insert_payment(receive_payment).await.unwrap();

        // Test filter by Send type only
        let send_only = storage
            .list_payments(ListPaymentsRequest {
                type_filter: Some(vec![PaymentType::Send]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(send_only.len(), 1);
        assert_eq!(send_only[0].id, "send_1");

        // Test filter by Receive type only
        let receive_only = storage
            .list_payments(ListPaymentsRequest {
                type_filter: Some(vec![PaymentType::Receive]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(receive_only.len(), 1);
        assert_eq!(receive_only[0].id, "receive_1");

        // Test filter by both types
        let both_types = storage
            .list_payments(ListPaymentsRequest {
                type_filter: Some(vec![PaymentType::Send, PaymentType::Receive]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(both_types.len(), 2);

        // Test with no filter (should return all)
        let all_payments = storage
            .list_payments(ListPaymentsRequest::default())
            .await
            .unwrap();
        assert_eq!(all_payments.len(), 2);
    }

    pub async fn test_payment_status_filtering(storage: Box<dyn Storage>) {
        // Create test payments with different statuses
        let completed_payment = Payment {
            id: "completed_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 10_000,
            fees: 100,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let pending_payment = Payment {
            id: "pending_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Pending,
            amount: 20_000,
            fees: 200,
            timestamp: 2000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let failed_payment = Payment {
            id: "failed_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Failed,
            amount: 30_000,
            fees: 300,
            timestamp: 3000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        storage.insert_payment(completed_payment).await.unwrap();
        storage.insert_payment(pending_payment).await.unwrap();
        storage.insert_payment(failed_payment).await.unwrap();

        // Test filter by Completed status only
        let completed_only = storage
            .list_payments(ListPaymentsRequest {
                status_filter: Some(vec![PaymentStatus::Completed]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(completed_only.len(), 1);
        assert_eq!(completed_only[0].id, "completed_1");

        // Test filter by Pending status only
        let pending_only = storage
            .list_payments(ListPaymentsRequest {
                status_filter: Some(vec![PaymentStatus::Pending]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(pending_only.len(), 1);
        assert_eq!(pending_only[0].id, "pending_1");

        // Test filter by multiple statuses
        let completed_or_failed = storage
            .list_payments(ListPaymentsRequest {
                status_filter: Some(vec![PaymentStatus::Completed, PaymentStatus::Failed]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(completed_or_failed.len(), 2);
    }

    #[allow(clippy::too_many_lines)]
    pub async fn test_asset_filtering(storage: Box<dyn Storage>) {
        use crate::models::TokenMetadata;

        // Create payments with different asset types
        let spark_payment = Payment {
            id: "spark_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 10_000,
            fees: 100,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let lightning_payment = Payment {
            id: "lightning_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 20_000,
            fees: 200,
            timestamp: 2000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                invoice: "lnbc1".to_string(),
                payment_hash: "hash1".to_string(),
                destination_pubkey: "pubkey1".to_string(),
                description: None,
                preimage: None,
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
            }),
        };

        let token_payment = Payment {
            id: "token_1".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 30_000,
            fees: 300,
            timestamp: 3000,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: TokenMetadata {
                    identifier: "token_id_1".to_string(),
                    issuer_public_key: "pubkey".to_string(),
                    name: "Token 1".to_string(),
                    ticker: "TK1".to_string(),
                    decimals: 8,
                    max_supply: 1_000_000,
                    is_freezable: false,
                },
                tx_hash: "tx_hash_1".to_string(),
                invoice_details: None,
            }),
        };

        let withdraw_payment = Payment {
            id: "withdraw_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 40_000,
            fees: 400,
            timestamp: 4000,
            method: PaymentMethod::Withdraw,
            details: Some(PaymentDetails::Withdraw {
                tx_id: "withdraw_tx_1".to_string(),
            }),
        };

        let deposit_payment = Payment {
            id: "deposit_1".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 50_000,
            fees: 500,
            timestamp: 5000,
            method: PaymentMethod::Deposit,
            details: Some(PaymentDetails::Deposit {
                tx_id: "deposit_tx_1".to_string(),
            }),
        };

        storage.insert_payment(spark_payment).await.unwrap();
        storage.insert_payment(lightning_payment).await.unwrap();
        storage.insert_payment(token_payment).await.unwrap();
        storage.insert_payment(withdraw_payment).await.unwrap();
        storage.insert_payment(deposit_payment).await.unwrap();

        // Test filter by Bitcoin
        let spark_only = storage
            .list_payments(ListPaymentsRequest {
                asset_filter: Some(crate::AssetFilter::Bitcoin),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(spark_only.len(), 4);

        // Test filter by Token (no identifier)
        let token_only = storage
            .list_payments(ListPaymentsRequest {
                asset_filter: Some(crate::AssetFilter::Token {
                    token_identifier: None,
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(token_only.len(), 1);
        assert_eq!(token_only[0].id, "token_1");

        // Test filter by Token with specific identifier
        let token_specific = storage
            .list_payments(ListPaymentsRequest {
                asset_filter: Some(crate::AssetFilter::Token {
                    token_identifier: Some("token_id_1".to_string()),
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(token_specific.len(), 1);
        assert_eq!(token_specific[0].id, "token_1");

        // Test filter by Token with non-existent identifier
        let token_no_match = storage
            .list_payments(ListPaymentsRequest {
                asset_filter: Some(crate::AssetFilter::Token {
                    token_identifier: Some("nonexistent".to_string()),
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(token_no_match.len(), 0);
    }

    pub async fn test_timestamp_filtering(storage: Box<dyn Storage>) {
        // Create payments at different timestamps
        let payment1 = Payment {
            id: "ts_1000".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 10_000,
            fees: 100,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let payment2 = Payment {
            id: "ts_2000".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 20_000,
            fees: 200,
            timestamp: 2000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let payment3 = Payment {
            id: "ts_3000".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 30_000,
            fees: 300,
            timestamp: 3000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        storage.insert_payment(payment1).await.unwrap();
        storage.insert_payment(payment2).await.unwrap();
        storage.insert_payment(payment3).await.unwrap();

        // Test filter by from_timestamp
        let from_2000 = storage
            .list_payments(ListPaymentsRequest {
                from_timestamp: Some(2000),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(from_2000.len(), 2);
        assert!(from_2000.iter().any(|p| p.id == "ts_2000"));
        assert!(from_2000.iter().any(|p| p.id == "ts_3000"));

        // Test filter by to_timestamp
        let to_2000 = storage
            .list_payments(ListPaymentsRequest {
                to_timestamp: Some(2000),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(to_2000.len(), 1);
        assert!(to_2000.iter().any(|p| p.id == "ts_1000"));

        // Test filter by both from_timestamp and to_timestamp
        let range = storage
            .list_payments(ListPaymentsRequest {
                from_timestamp: Some(1500),
                to_timestamp: Some(2500),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].id, "ts_2000");
    }

    pub async fn test_combined_filters(storage: Box<dyn Storage>) {
        // Create diverse test payments
        let payment1 = Payment {
            id: "combined_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 10_000,
            fees: 100,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let payment2 = Payment {
            id: "combined_2".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Pending,
            amount: 20_000,
            fees: 200,
            timestamp: 2000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                invoice: "lnbc1".to_string(),
                payment_hash: "hash1".to_string(),
                destination_pubkey: "pubkey1".to_string(),
                description: None,
                preimage: None,
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
            }),
        };

        let payment3 = Payment {
            id: "combined_3".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 30_000,
            fees: 300,
            timestamp: 3000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                invoice: "lnbc2".to_string(),
                payment_hash: "hash2".to_string(),
                destination_pubkey: "pubkey2".to_string(),
                description: None,
                preimage: None,
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
            }),
        };

        storage.insert_payment(payment1).await.unwrap();
        storage.insert_payment(payment2).await.unwrap();
        storage.insert_payment(payment3).await.unwrap();

        // Test: Send + Completed
        let send_completed = storage
            .list_payments(ListPaymentsRequest {
                type_filter: Some(vec![PaymentType::Send]),
                status_filter: Some(vec![PaymentStatus::Completed]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(send_completed.len(), 1);
        assert_eq!(send_completed[0].id, "combined_1");

        // Test: Bitcoin + timestamp range
        let bitcoin_recent = storage
            .list_payments(ListPaymentsRequest {
                asset_filter: Some(crate::AssetFilter::Bitcoin),
                from_timestamp: Some(2500),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(bitcoin_recent.len(), 1);
        assert_eq!(bitcoin_recent[0].id, "combined_3");

        // Test: Type + Status + Asset
        let send_pending_bitcoin = storage
            .list_payments(ListPaymentsRequest {
                type_filter: Some(vec![PaymentType::Send]),
                status_filter: Some(vec![PaymentStatus::Pending]),
                asset_filter: Some(crate::AssetFilter::Bitcoin),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(send_pending_bitcoin.len(), 1);
        assert_eq!(send_pending_bitcoin[0].id, "combined_2");
    }

    pub async fn test_sort_order(storage: Box<dyn Storage>) {
        // Create payments at different timestamps
        let payment1 = Payment {
            id: "sort_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 10_000,
            fees: 100,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let payment2 = Payment {
            id: "sort_2".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 20_000,
            fees: 200,
            timestamp: 2000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        let payment3 = Payment {
            id: "sort_3".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 30_000,
            fees: 300,
            timestamp: 3000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
            }),
        };

        storage.insert_payment(payment1).await.unwrap();
        storage.insert_payment(payment2).await.unwrap();
        storage.insert_payment(payment3).await.unwrap();

        // Test default sort (descending by timestamp)
        let desc_payments = storage
            .list_payments(ListPaymentsRequest::default())
            .await
            .unwrap();
        assert_eq!(desc_payments.len(), 3);
        assert_eq!(desc_payments[0].id, "sort_3"); // Most recent first
        assert_eq!(desc_payments[1].id, "sort_2");
        assert_eq!(desc_payments[2].id, "sort_1");

        // Test ascending sort
        let asc_payments = storage
            .list_payments(ListPaymentsRequest {
                sort_ascending: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(asc_payments.len(), 3);
        assert_eq!(asc_payments[0].id, "sort_1"); // Oldest first
        assert_eq!(asc_payments[1].id, "sort_2");
        assert_eq!(asc_payments[2].id, "sort_3");

        // Test explicit descending sort
        let desc_explicit = storage
            .list_payments(ListPaymentsRequest {
                sort_ascending: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(desc_explicit.len(), 3);
        assert_eq!(desc_explicit[0].id, "sort_3");
        assert_eq!(desc_explicit[1].id, "sort_2");
        assert_eq!(desc_explicit[2].id, "sort_1");
    }

    pub async fn test_payment_request_metadata(storage: Box<dyn Storage>) {
        let cache = ObjectCacheRepository::new(storage.into());

        // Prepare test data
        let payment_request1 = "pr1".to_string();
        let metadata1 = PaymentRequestMetadata {
            payment_request: payment_request1.clone(),
            lnurl_withdraw_request_details: LnurlWithdrawRequestDetails {
                callback: "https://callback.url".to_string(),
                k1: "k1value".to_string(),
                default_description: "desc1".to_string(),
                min_withdrawable: 1000,
                max_withdrawable: 2000,
            },
        };

        let payment_request2 = "pr2".to_string();
        let metadata2 = PaymentRequestMetadata {
            payment_request: payment_request2.clone(),
            lnurl_withdraw_request_details: LnurlWithdrawRequestDetails {
                callback: "https://callback2.url".to_string(),
                k1: "k1value2".to_string(),
                default_description: "desc2".to_string(),
                min_withdrawable: 10000,
                max_withdrawable: 20000,
            },
        };

        // set_payment_request_metadata
        cache
            .save_payment_request_metadata(&metadata1)
            .await
            .unwrap();
        cache
            .save_payment_request_metadata(&metadata2)
            .await
            .unwrap();

        // get_payment_request_metadata
        let fetched1 = cache
            .fetch_payment_request_metadata(&payment_request1)
            .await
            .unwrap();
        assert!(fetched1.is_some());
        let fetched1 = fetched1.unwrap();
        assert_eq!(fetched1.payment_request, payment_request1);
        // Check lnurl_withdraw_request_details is present and correct
        let details = fetched1.lnurl_withdraw_request_details;
        assert_eq!(details.k1, "k1value");
        assert_eq!(details.default_description, "desc1");
        assert_eq!(details.min_withdrawable, 1000);
        assert_eq!(details.max_withdrawable, 2000);
        assert_eq!(details.callback, "https://callback.url");

        let fetched2 = cache
            .fetch_payment_request_metadata(&payment_request2)
            .await
            .unwrap();
        assert!(fetched2.is_some());
        assert_eq!(fetched2.as_ref().unwrap().payment_request, payment_request2);

        // delete_payment_request_metadata
        cache
            .delete_payment_request_metadata(&payment_request1)
            .await
            .unwrap();
        let deleted = cache
            .fetch_payment_request_metadata(&payment_request1)
            .await
            .unwrap();
        assert!(deleted.is_none());
    }
}
