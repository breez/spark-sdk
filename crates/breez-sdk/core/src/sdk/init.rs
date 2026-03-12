use flashnet::{FlashnetConfig, IntegratorConfig};
use std::sync::Arc;
use tokio::sync::{OnceCell, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{Instrument, debug, error, info, warn};

use crate::{
    Network,
    error::SdkError,
    persist::ObjectCacheRepository,
    stable_balance::StableBalance,
    token_conversion::{
        DEFAULT_INTEGRATOR_FEE_BPS, DEFAULT_INTEGRATOR_PUBKEY, FlashnetTokenConverter,
        TokenConverter,
    },
};

use super::{BreezSdk, BreezSdkParams, SyncCoordinator, helpers::validate_breez_api_key};

impl BreezSdk {
    /// Creates a new instance of the `BreezSdk`
    pub(crate) fn init_and_start(params: BreezSdkParams) -> Result<Self, SdkError> {
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

        // Create the FlashnetTokenConverter (spawns its own refunder background task)
        let flashnet_config = FlashnetConfig::default_config(
            params.config.network.into(),
            DEFAULT_INTEGRATOR_PUBKEY
                .parse()
                .ok()
                .map(|pubkey| IntegratorConfig {
                    pubkey,
                    fee_bps: DEFAULT_INTEGRATOR_FEE_BPS,
                }),
        );
        let token_converter: Arc<dyn TokenConverter> = Arc::new(FlashnetTokenConverter::new(
            flashnet_config,
            Arc::clone(&params.storage),
            Arc::clone(&params.spark_wallet),
            params.config.network,
            params.shutdown_sender.subscribe(),
        ));

        // Create StableBalance if configured (spawns its own auto-convert background task)
        let stable_balance = params.config.stable_balance_config.as_ref().map(|config| {
            Arc::new(StableBalance::new(
                config.clone(),
                Arc::clone(&token_converter),
                Arc::clone(&params.spark_wallet),
                params.shutdown_sender.subscribe(),
                params.sync_signing_client.clone(),
            ))
        });
        let sync_coordinator = SyncCoordinator::new();

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
            sync_coordinator,
            lnurl_preimage_trigger: tokio::sync::broadcast::channel(10).0,
            initial_synced_watcher,
            external_input_parsers,
            spark_private_mode_initialized: Arc::new(OnceCell::new()),
            token_converter,
            stable_balance,
            buy_bitcoin_provider: params.buy_bitcoin_provider,
        };

        sdk.start(initial_synced_sender);
        Ok(sdk)
    }

    /// Starts the SDK's background tasks
    ///
    /// This method initiates the following background tasks:
    /// 1. `spawn_spark_private_mode_initialization`: initializes the spark private mode on startup
    /// 2. `periodic_sync`: syncs the wallet with the Spark network
    /// 3. `try_recover_lightning_address`: recovers the lightning address on startup
    /// 4. `spawn_lnurl_preimage_publisher`: publishes lnurl preimages for completed payments
    pub(super) fn start(&self, initial_synced_sender: watch::Sender<bool>) {
        self.spawn_spark_private_mode_initialization();
        self.periodic_sync(initial_synced_sender);
        self.try_recover_lightning_address();
        self.spawn_lnurl_preimage_publisher();
    }

    fn spawn_spark_private_mode_initialization(&self) {
        let sdk = self.clone();
        let span = tracing::Span::current();
        tokio::spawn(
            async move {
                if let Err(e) = sdk.ensure_spark_private_mode_initialized().await {
                    error!("Failed to initialize spark private mode: {e:?}");
                }
            }
            .instrument(span),
        );
    }

    /// Refreshes the user's lightning address on the server on startup.
    ///
    /// When real-time sync is enabled, recovery is normally handled by incoming
    /// sync updates. However, we still recover here if the webhook secret hasn't
    /// been configured yet (e.g. registrations made before webhooks were introduced).
    fn try_recover_lightning_address(&self) {
        let sdk = self.clone();
        let span = tracing::Span::current();
        tokio::spawn(async move {
            if sdk.config.lnurl_domain.is_none() {
                return;
            }

            if sdk.config.real_time_sync_server_url.is_some() {
                let cache = ObjectCacheRepository::new(sdk.storage.clone());
                match cache.fetch_webhook_configured().await {
                    Ok(true) => return,
                    Ok(false) => debug!("webhook not configured, recovering lightning address despite real-time sync"),
                    Err(e) => {
                        warn!("failed to check webhook configured flag: {e:?}");
                        return;
                    }
                }
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

    pub(super) async fn ensure_spark_private_mode_initialized(&self) -> Result<(), SdkError> {
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
        })
        .await?;
        ObjectCacheRepository::new(self.storage.clone())
            .save_spark_private_mode_initialized()
            .await?;
        info!("Spark private mode initialized: enabled");
        Ok(())
    }
}
