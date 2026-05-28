use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    BitcoinChainService, BreezSdk, Config, Credentials, FiatService, KeySetConfig, PaymentObserver,
    RestClient, SdkContext, SdkError, Seed, Storage, StorageBackend,
    chain::rest_client::ChainApiType,
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
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(config: Config, seed: Seed) -> Self {
        let inner = crate::sdk_builder::SdkBuilder::new(config, seed);
        SdkBuilder {
            inner: Mutex::new(inner),
        }
    }

    /// Sets the root storage directory to initialize the default storage with.
    /// This initializes both storage and real-time sync storage with the
    /// default implementations.
    /// Arguments:
    /// - `storage_dir`: The data directory for storage.
    #[cfg(feature = "sqlite")]
    pub async fn with_default_storage(&self, storage_dir: String) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_default_storage(storage_dir);
    }

    /// Sets the storage backend to be used by the SDK.
    ///
    /// Build the [`StorageBackend`](crate::StorageBackend) via
    /// [`default_storage`](crate::default_storage),
    /// [`postgres_storage`](crate::postgres_storage),
    /// [`mysql_storage`](crate::mysql_storage) or
    /// [`custom_storage`](crate::custom_storage).
    /// Arguments:
    /// - `storage`: The storage backend to be used.
    pub async fn with_storage_backend(&self, storage: Arc<dyn StorageBackend>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_storage_backend(storage);
    }

    /// **Deprecated.** Use
    /// [`with_storage_backend`](SdkBuilder::with_storage_backend) with
    /// [`custom_storage`](crate::custom_storage).
    /// Arguments:
    /// - `storage`: The storage implementation to be used.
    #[allow(deprecated)]
    pub async fn with_storage(&self, storage: Arc<dyn Storage>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_storage(storage);
    }

    /// Sets the key set type to be used by the SDK.
    /// Arguments:
    /// - `config`: Key set configuration containing the key set type, address index flag, and optional account number.
    pub async fn with_key_set(&self, config: KeySetConfig) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_key_set(config);
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
    /// - `api_type`: The API type to be used.
    /// - `credentials`: Optional credentials for basic authentication.
    pub async fn with_rest_chain_service(
        &self,
        url: String,
        api_type: ChainApiType,
        credentials: Option<Credentials>,
    ) {
        let mut builder = self.inner.lock().await;
        *builder = builder
            .clone()
            .with_rest_chain_service(url, api_type, credentials);
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

    /// Threads a shared [`SdkContext`](crate::SdkContext) into the builder.
    ///
    /// Construct the context once via
    /// [`new_shared_sdk_context`](crate::new_shared_sdk_context) and pass the
    /// same `Arc` to every `SdkBuilder` whose SDKs should share its resources
    /// (operator gRPC channels, SSP HTTP client, database pool).
    pub async fn with_shared_context(&self, context: Arc<SdkContext>) {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_shared_context(context);
    }

    /// Builds the `BreezSdk` instance with the configured components.
    pub async fn build(&self) -> Result<BreezSdk, SdkError> {
        self.inner.lock().await.clone().build().await
    }
}

#[cfg(feature = "postgres")]
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl SdkBuilder {
    /// **Deprecated.** Use [`with_storage`](SdkBuilder::with_storage) with
    /// [`postgres_storage`](crate::postgres_storage).
    #[allow(deprecated)]
    pub async fn with_postgres_backend(
        &self,
        config: crate::persist::postgres::PostgresStorageConfig,
    ) -> Result<(), SdkError> {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_postgres_backend(config)?;
        Ok(())
    }
}

#[cfg(feature = "mysql")]
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl SdkBuilder {
    /// **Deprecated.** Use [`with_storage`](SdkBuilder::with_storage) with
    /// [`mysql_storage`](crate::mysql_storage).
    #[allow(deprecated)]
    pub async fn with_mysql_backend(
        &self,
        config: crate::persist::mysql::MysqlStorageConfig,
    ) -> Result<(), SdkError> {
        let mut builder = self.inner.lock().await;
        *builder = builder.clone().with_mysql_backend(config)?;
        Ok(())
    }
}
