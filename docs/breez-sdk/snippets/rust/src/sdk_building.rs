use async_trait::async_trait;
use breez_sdk_spark::*;
use anyhow::Result;
use log::info;

pub(crate) async fn init_sdk_advanced() -> Result<BreezSdk> {
    // ANCHOR: init-sdk-advanced
    // Construct the seed using mnemonic words or entropy bytes
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Build the SDK using the config, seed and default storage
    let builder = SdkBuilder::new(config, seed);
    builder.with_default_storage("./.data".to_string());
    // You can also pass your custom implementations:
    // let builder = builder.with_storage(<your storage implementation>)
    // let builder = builder.with_real_time_sync_storage(<your real-time sync storage implementation>)
    // let builder = builder.with_chain_service(<your chain service implementation>)
    // let builder = builder.with_rest_client(<your rest client implementation>)
    // let builder = builder.with_key_set(<your key set type>, <use address index>, <account number>)
    // let builder = builder.with_payment_observer(<your payment observer implementation>);
    let sdk = builder.build().await?;

    // ANCHOR_END: init-sdk-advanced
    Ok(sdk)
}

pub(crate) fn with_rest_chain_service(builder: &mut SdkBuilder) {
    // ANCHOR: with-rest-chain-service
    let url = "<your REST chain service URL>".to_string();
    let chain_api_type = ChainApiType::MempoolSpace;
    let optional_credentials = Credentials {
        username: "<username>".to_string(),
        password: "<password>".to_string(),
    };
    builder.with_rest_chain_service(
        url,
        chain_api_type,
        Some(optional_credentials),
    )
    // ANCHOR_END: with-rest-chain-service
}

pub(crate) fn with_key_set(builder: &mut SdkBuilder) {
    // ANCHOR: with-key-set
    let key_set_type = KeySetType::Default;
    let use_address_index = false;
    let optional_account_number = 21;
    builder.with_key_set(key_set_type, use_address_index, Some(optional_account_number));
    // ANCHOR_END: with-key-set
}

// ANCHOR: with-storage
#[async_trait]
pub trait Storage: Send + Sync {
    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError>;
    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError>;
    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError>;
    async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<Vec<Payment>, StorageError>;
    async fn insert_payment(&self, payment: Payment) -> Result<(), StorageError>;
    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), StorageError>;
    async fn get_payment_by_id(&self, id: String) -> Result<Payment, StorageError>;
    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<Payment>, StorageError>;
    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError>;
    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError>;
    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError>;
    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), StorageError>;
}
// ANCHOR_END: with-storage

// ANCHOR: with-sync-storage
#[async_trait]
pub trait SyncStorage: Send + Sync {
    async fn add_outgoing_change(
        &self,
        record: UnversionedRecordChange,
    ) -> Result<u64, SyncStorageError>;
    async fn complete_outgoing_sync(&self, record: Record) -> Result<(), SyncStorageError>;
    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<OutgoingChange>, SyncStorageError>;
    async fn get_last_revision(&self) -> Result<u64, SyncStorageError>;
    async fn insert_incoming_records(&self, records: Vec<Record>) -> Result<(), SyncStorageError>;
    async fn delete_incoming_record(&self, record: Record) -> Result<(), SyncStorageError>;
    async fn rebase_pending_outgoing_records(&self, revision: u64) -> Result<(), SyncStorageError>;
    async fn get_incoming_records(
        &self,
        limit: u32,
    ) -> Result<Vec<IncomingChange>, SyncStorageError>;
    async fn get_latest_outgoing_change(&self) -> Result<Option<OutgoingChange>, SyncStorageError>;
    async fn update_record_from_incoming(&self, record: Record) -> Result<(), SyncStorageError>;
}
// ANCHOR_END: with-sync-storage

// ANCHOR: with-chain-service
#[async_trait]
pub trait BitcoinChainService: Send + Sync {
    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError>;
    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError>;
    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError>;
    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError>;
}
// ANCHOR_END: with-chain-service

// ANCHOR: with-rest-client
#[async_trait]
pub trait RestClient: Send + Sync {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<RestResponse, ServiceConnectivityError>;
    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError>;
    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<RestResponse, ServiceConnectivityError>;
}
// ANCHOR_END: with-rest-client

// ANCHOR: with-fiat-service
#[async_trait]
pub trait FiatService: Send + Sync {
    async fn fetch_fiat_currencies(&self) -> Result<Vec<FiatCurrency>, ServiceConnectivityError>;
    async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError>;
}
// ANCHOR_END: with-fiat-service

// ANCHOR: with-payment-observer
#[async_trait]
pub trait PaymentObserver: Send + Sync {
    async fn before_send(&self, payments: Vec<ProvisionalPayment>) -> Result<(), PaymentObserverError>;
}
// ANCHOR_END: with-payment-observer
