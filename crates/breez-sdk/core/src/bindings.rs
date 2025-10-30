use std::sync::Arc;

use breez_sdk_common::sync::storage::SyncStorage;
use breez_sdk_common::{fiat::FiatService, rest::RestClient};
use tokio::sync::Mutex;

use crate::sdk_builder::Seed;
use crate::{
    BitcoinChainService, BreezSdk, Config, Credentials, KeySetType, PaymentObserver, SdkError,
    Storage,
};

/// Builder for creating `BreezSdk` instances with customizable components.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SdkBuilder {
    inner: Mutex<crate::sdk_builder::SdkBuilder>,
}

/// Builder for creating `BreezSdk` instances with customizable components.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl SdkBuilder {
    /// Creates a new `SdkBuilder` with the provided configuration.
    /// Arguments:
    /// - `config`: The configuration to be used.
    /// - `seed`: The seed for wallet generation.
    /// - `storage`: The storage backend to be used.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(config: Config, seed: Seed, storage: Arc<dyn Storage>) -> Self {
        let inner = crate::sdk_builder::SdkBuilder::new(config, seed, storage);
        SdkBuilder {
            inner: Mutex::new(inner),
        }
    }

    /// Sets the key set type to be used by the SDK.
    /// Arguments:
    /// - `key_set_type`: The key set type which determines the derivation path.
    /// - `use_address_index`: Controls the structure of the BIP derivation path.
    pub async fn with_key_set(
        &self,
        key_set_type: KeySetType,
        use_address_index: bool,
        account_number: Option<u32>,
    ) {
        let mut builder = self.inner.lock().await;
        *builder = builder
            .clone()
            .with_key_set(key_set_type, use_address_index, account_number);
    }

    /// Sets the chain service to be used by the SDK.
    /// Arguments:
    /// - `chain_service`: The chain service to be used.
    pub async fn with_chain_service(&self, chain_service: Arc<dyn BitcoinChainService>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_chain_service(chain_service);
    }

    /// Sets the REST chain service to be used by the SDK.
    /// Arguments:
    /// - `url`: The base URL of the REST API.
    /// - `credentials`: Optional credentials for basic authentication.
    pub async fn with_rest_chain_service(&self, url: String, credentials: Option<Credentials>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_rest_chain_service(url, credentials);
    }

    /// Sets the fiat service to be used by the SDK.
    /// Arguments:
    /// - `fiat_service`: The fiat service to be used.
    pub async fn with_fiat_service(&self, fiat_service: Arc<dyn FiatService>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_fiat_service(fiat_service);
    }

    pub async fn with_lnurl_client(&self, lnurl_client: Arc<dyn RestClient>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_lnurl_client(lnurl_client);
    }

    /// Sets the payment observer to be used by the SDK.
    /// Arguments:
    /// - `payment_observer`: The payment observer to be used.
    pub async fn with_payment_observer(&self, payment_observer: Arc<dyn PaymentObserver>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_payment_observer(payment_observer);
    }

    pub async fn with_real_time_sync_storage(&self, storage: Arc<dyn SyncStorage>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_real_time_sync_storage(storage);
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(&self) -> Result<BreezSdk, SdkError> {
        self.inner.lock().await.clone().build().await
    }
}
