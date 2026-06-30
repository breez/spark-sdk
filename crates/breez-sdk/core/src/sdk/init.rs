use platform_utils::tokio;
use std::sync::Arc;
use tokio::sync::{OnceCell, watch};
use tracing::{Instrument, error, info};

use crate::{Network, error::SdkError, persist::ObjectCacheRepository};

use super::{BreezSdk, BreezSdkParams, helpers::validate_breez_api_key};

impl BreezSdk {
    /// Creates a new instance of the `BreezSdk`
    pub(crate) async fn init_and_start(params: BreezSdkParams) -> Result<Self, SdkError> {
        // In Regtest we allow running without a Breez API key to facilitate local
        // integration tests. For non-regtest networks, a valid API key is required.
        if !matches!(params.config.network, Network::Regtest) {
            match &params.config.api_key {
                Some(api_key) => validate_breez_api_key(api_key)?,
                None => return Err(SdkError::Generic("Missing Breez API key".to_string())),
            }
        }
        let (initial_synced_sender, initial_synced_watcher) = watch::channel(false);
        let external_input_parsers = params.config.get_all_external_input_parsers();

        let sdk = Self {
            config: params.config,
            spark_wallet: params.spark_wallet,
            storage: params.storage,
            chain_service: params.chain_service,
            fiat_service: params.fiat_service,
            lnurl_client: params.lnurl_client,
            lnurl_server_client: params.lnurl_server_client,
            lnurl_auth_signer: params.lnurl_auth_signer,
            event_emitter: params.event_emitter,
            shutdown_sender: params.shutdown_sender,
            runtime: params.runtime,
            sync_coordinator: params.sync_coordinator,
            initial_synced_watcher,
            external_input_parsers,
            spark_private_mode_initialized: Arc::new(OnceCell::new()),
            token_converter: params.token_converter,
            stable_balance: params.stable_balance,
            buy_bitcoin_provider: params.buy_bitcoin_provider,
            cross_chain_context: params.cross_chain_context,
            lightning_sender: params.lightning_sender,
        };

        sdk.start(initial_synced_sender).await;
        Ok(sdk)
    }

    /// Starts the SDK runtime services selected during construction.
    pub(super) async fn start(&self, initial_synced_sender: watch::Sender<bool>) {
        self.runtime
            .start_sdk_services(self, initial_synced_sender)
            .await;
    }

    pub(crate) fn spawn_spark_private_mode_initialization(&self) {
        let sdk = self.clone();
        let span = tracing::Span::current();
        tokio::spawn(
            async move {
                if let Err(e) = sdk.maybe_ensure_spark_private_mode_initialized().await {
                    error!("Failed to initialize spark private mode: {e:?}");
                }
            }
            .instrument(span),
        );
    }

    /// Refreshes the user's lightning address on the server on startup.
    pub(crate) fn try_recover_lightning_address(&self) {
        let sdk = self.clone();
        let span = tracing::Span::current();
        tokio::spawn(async move {
            if sdk.config.lnurl_domain.is_none() {
                return;
            }

            match sdk.recover_lightning_address().await {
                Ok(None) => info!("no lightning address to recover on startup"),
                Ok(Some(value)) => info!(
                    "recovered lightning address on startup: address: {}, lnurl url: {}, lnurl bech32: {}",
                    value.lightning_address, value.lnurl.url, value.lnurl.bech32
                ),
                Err(e) => error!("Failed to recover lightning address on startup: {e:?}"),
            }
        }.instrument(span));
    }

    pub(super) async fn maybe_ensure_spark_private_mode_initialized(&self) -> Result<(), SdkError> {
        self.runtime
            .maybe_ensure_spark_private_mode_initialized(self)
            .await
    }

    pub(super) async fn ensure_spark_private_mode_initialized_inner(&self) -> Result<(), SdkError> {
        self.spark_private_mode_initialized
            .get_or_try_init(|| async {
                // Check if already initialized in storage
                let object_repository = ObjectCacheRepository::new(self.storage.clone());
                let is_initialized = object_repository
                    .fetch_spark_private_mode_initialized()
                    .await?;

                if !is_initialized {
                    // Initialize if not already done
                    self.initialize_spark_private_mode().await?;
                }
                Ok::<_, SdkError>(())
            })
            .await?;
        Ok(())
    }

    async fn initialize_spark_private_mode(&self) -> Result<(), SdkError> {
        if !self.config.private_enabled_default {
            ObjectCacheRepository::new(self.storage.clone())
                .save_spark_private_mode_initialized()
                .await?;
            info!("Spark private mode initialized: no changes needed");
            return Ok(());
        }

        // Enable spark private mode
        self.update_user_settings(crate::UpdateUserSettingsRequest {
            spark_private_mode_enabled: Some(true),
            stable_balance_active_label: None,
        })
        .await?;
        ObjectCacheRepository::new(self.storage.clone())
            .save_spark_private_mode_initialized()
            .await?;
        info!("Spark private mode initialized: enabled");
        Ok(())
    }
}
