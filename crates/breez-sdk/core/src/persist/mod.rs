#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub(crate) mod sqlite;

use std::{collections::HashMap, sync::Arc};

use macros::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    DepositClaimError, DepositInfo, LightningAddressInfo, LnurlPayInfo, TokenBalance,
    models::Payment,
};

const ACCOUNT_INFO_KEY: &str = "account_info";
const LIGHTNING_ADDRESS_KEY: &str = "lightning_address";
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
    pub lnurl_description: Option<String>,
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
    pub(crate) last_synced_token_payment_id: Option<String>,
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
    use chrono::Utc;

    use crate::{
        DepositClaimError, Payment, PaymentDetails, PaymentMetadata, PaymentMethod, PaymentStatus,
        PaymentType, Storage, UpdateDepositPayload,
    };

    #[allow(clippy::too_many_lines)]
    pub async fn test_sqlite_storage(storage: Box<dyn Storage>) {
        use crate::models::{LnurlPayInfo, TokenMetadata};

        // Test 1: Spark payment
        let spark_payment = Payment {
            id: "spark_pmt123".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 100_000,
            fees: 1000,
            timestamp: 5000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark),
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
            }),
        };

        // Test 3: Lightning payment with full details
        let metadata = PaymentMetadata {
            lnurl_pay_info: Some(LnurlPayInfo {
                ln_address: Some("test@example.com".to_string()),
                comment: Some("Test comment".to_string()),
                domain: Some("example.com".to_string()),
                metadata: Some("[[\"text/plain\", \"Test metadata\"]]".to_string()),
                processed_success_action: None,
                raw_success_action: None,
            }),
            lnurl_description: None,
        };
        let lightning_payment = Payment {
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
                lnurl_pay_info: metadata.lnurl_pay_info.clone(),
            }),
        };

        // Test 4: Lightning payment with minimal details
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
            }),
        };

        // Test 5: Withdraw payment
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

        // Test 6: Deposit payment
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

        // Test 7: Payment with no details
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
            lightning_payment.clone(),
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
            .set_payment_metadata(lightning_payment.id.clone(), metadata)
            .await
            .unwrap();

        // List all payments
        let payments = storage.list_payments(Some(0), Some(10)).await.unwrap();
        assert_eq!(payments.len(), 7);

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
                (Some(PaymentDetails::Spark), Some(PaymentDetails::Spark)) | (None, None) => {}
                (
                    Some(PaymentDetails::Token {
                        metadata: retrieved_metadata,
                        tx_hash: retrieved_tx_hash,
                    }),
                    Some(PaymentDetails::Token {
                        metadata: expected_metadata,
                        tx_hash: expected_tx_hash,
                    }),
                ) => {
                    assert_eq!(retrieved_metadata.identifier, expected_metadata.identifier);
                    assert_eq!(
                        retrieved_metadata.issuer_public_key,
                        expected_metadata.issuer_public_key
                    );
                    assert_eq!(retrieved_metadata.name, expected_metadata.name);
                    assert_eq!(retrieved_metadata.ticker, expected_metadata.ticker);
                    assert_eq!(retrieved_metadata.decimals, expected_metadata.decimals);
                    assert_eq!(retrieved_metadata.max_supply, expected_metadata.max_supply);
                    assert_eq!(
                        retrieved_metadata.is_freezable,
                        expected_metadata.is_freezable
                    );
                    assert_eq!(retrieved_tx_hash, expected_tx_hash);
                }
                (
                    Some(PaymentDetails::Lightning {
                        description: r_description,
                        preimage: r_preimage,
                        invoice: r_invoice,
                        payment_hash: r_hash,
                        destination_pubkey: r_dest_pubkey,
                        lnurl_pay_info: r_lnurl,
                    }),
                    Some(PaymentDetails::Lightning {
                        description: e_description,
                        preimage: e_preimage,
                        invoice: e_invoice,
                        payment_hash: e_hash,
                        destination_pubkey: e_dest_pubkey,
                        lnurl_pay_info: e_lnurl,
                    }),
                ) => {
                    assert_eq!(r_description, e_description);
                    assert_eq!(r_preimage, e_preimage);
                    assert_eq!(r_invoice, e_invoice);
                    assert_eq!(r_hash, e_hash);
                    assert_eq!(r_dest_pubkey, e_dest_pubkey);

                    // Test LNURL pay info if present
                    match (r_lnurl, e_lnurl) {
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
        assert_eq!(send_payments, 4); // spark, lightning, withdraw, no_details
        assert_eq!(receive_payments, 3); // token, lightning_minimal, deposit

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
        assert_eq!(completed_payments, 4); // spark, lightning, withdraw, deposit
        assert_eq!(pending_payments, 2); // token, no_details
        assert_eq!(failed_payments, 1); // lightning_minimal

        // Test filtering by method
        let lightning_count = payments
            .iter()
            .filter(|p| p.method == PaymentMethod::Lightning)
            .count();
        assert_eq!(lightning_count, 2); // lightning and lightning_minimal
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
