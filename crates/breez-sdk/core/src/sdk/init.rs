use flashnet::{FlashnetConfig, IntegratorConfig};
use std::sync::Arc;
use tokio::sync::{OnceCell, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{error, info};

use crate::{
    AssetFilter, Network, PaymentDetails, PaymentStatus, PaymentType, SetLnurlMetadataItem,
    error::SdkError,
    lnurl::PublishZapReceiptRequest,
    models::ListPaymentsRequest,
    persist::ObjectCacheRepository,
    stable_balance::StableBalance,
    token_conversion::{
        DEFAULT_INTEGRATOR_FEE_BPS, DEFAULT_INTEGRATOR_PUBKEY, FlashnetTokenConverter,
        TokenConverter,
    },
};

use super::{BreezSdk, BreezSdkParams, helpers::validate_breez_api_key};

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
            sync_trigger: tokio::sync::broadcast::channel(10).0,
            zap_receipt_trigger: tokio::sync::broadcast::channel(10).0,
            initial_synced_watcher,
            external_input_parsers,
            spark_private_mode_initialized: Arc::new(OnceCell::new()),
            nostr_client: params.nostr_client,
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
    /// 4. `spawn_zap_receipt_publisher`: publishes zap receipts for payments with zap requests
    pub(super) fn start(&self, initial_synced_sender: watch::Sender<bool>) {
        self.spawn_spark_private_mode_initialization();
        self.periodic_sync(initial_synced_sender);
        self.try_recover_lightning_address();
        self.spawn_zap_receipt_publisher();
    }

    fn spawn_spark_private_mode_initialization(&self) {
        let sdk = self.clone();
        tokio::spawn(async move {
            if let Err(e) = sdk.ensure_spark_private_mode_initialized().await {
                error!("Failed to initialize spark private mode: {e:?}");
            }
        });
    }

    /// Refreshes the user's lightning address on the server on startup.
    fn try_recover_lightning_address(&self) {
        let sdk = self.clone();
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
        });
    }

    /// Background task that publishes zap receipts for payments with zap requests.
    /// Triggered on startup and after syncing lnurl metadata.
    fn spawn_zap_receipt_publisher(&self) {
        let sdk = self.clone();
        let mut shutdown_receiver = sdk.shutdown_sender.subscribe();
        let mut trigger_receiver = sdk.zap_receipt_trigger.clone().subscribe();

        tokio::spawn(async move {
            if let Err(e) = Self::process_pending_zap_receipts(&sdk).await {
                error!("Failed to process pending zap receipts on startup: {e:?}");
            }

            loop {
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Zap receipt publisher shutdown signal received");
                        return;
                    }
                    _ = trigger_receiver.recv() => {
                        if let Err(e) = Self::process_pending_zap_receipts(&sdk).await {
                            error!("Failed to process pending zap receipts: {e:?}");
                        }
                    }
                }
            }
        });
    }

    pub(super) async fn process_pending_zap_receipts(&self) -> Result<(), SdkError> {
        let Some(lnurl_server_client) = self.lnurl_server_client.clone() else {
            return Ok(());
        };

        let mut offset = 0;
        let limit = 100;
        loop {
            let payments = self
                .storage
                .list_payments(ListPaymentsRequest {
                    offset: Some(offset),
                    limit: Some(limit),
                    status_filter: Some(vec![PaymentStatus::Completed]),
                    type_filter: Some(vec![PaymentType::Receive]),
                    asset_filter: Some(AssetFilter::Bitcoin),
                    ..Default::default()
                })
                .await?;
            if payments.is_empty() {
                break;
            }

            let len = u32::try_from(payments.len())?;
            for payment in payments {
                let Some(PaymentDetails::Lightning {
                    ref lnurl_receive_metadata,
                    ref payment_hash,
                    ..
                }) = payment.details
                else {
                    continue;
                };

                let Some(lnurl_receive_metadata) = lnurl_receive_metadata else {
                    continue;
                };

                let Some(zap_request) = &lnurl_receive_metadata.nostr_zap_request else {
                    continue;
                };

                if lnurl_receive_metadata.nostr_zap_receipt.is_some() {
                    continue;
                }

                // Create the zap receipt using NostrClient
                let zap_receipt = match self
                    .nostr_client
                    .create_zap_receipt(zap_request, &payment)
                    .await
                {
                    Ok(receipt) => receipt,
                    Err(e) => {
                        error!(
                            "Failed to create zap receipt for payment {}: {e:?}",
                            payment.id
                        );
                        continue;
                    }
                };

                // Publish the zap receipt via the server
                let zap_receipt = match lnurl_server_client
                    .publish_zap_receipt(&PublishZapReceiptRequest {
                        payment_hash: payment_hash.clone(),
                        zap_receipt: zap_receipt.clone(),
                    })
                    .await
                {
                    Ok(zap_receipt) => zap_receipt,
                    Err(e) => {
                        error!(
                            "Failed to publish zap receipt for payment {}: {}",
                            payment.id, e
                        );
                        continue;
                    }
                };

                if let Err(e) = self
                    .storage
                    .set_lnurl_metadata(vec![SetLnurlMetadataItem {
                        sender_comment: lnurl_receive_metadata.sender_comment.clone(),
                        nostr_zap_request: Some(zap_request.clone()),
                        nostr_zap_receipt: Some(zap_receipt),
                        payment_hash: payment_hash.clone(),
                    }])
                    .await
                {
                    error!(
                        "Failed to store zap receipt for payment {}: {}",
                        payment.id, e
                    );
                }
            }

            if len < limit {
                break;
            }

            offset = offset.saturating_add(len);
        }

        Ok(())
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
