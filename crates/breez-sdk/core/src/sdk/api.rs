use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use breez_sdk_common::buy::cashapp::CashAppProvider;
use std::str::FromStr;
use tracing::{debug, info};

use crate::{
    BuyBitcoinRequest, BuyBitcoinResponse, CheckMessageRequest, CheckMessageResponse,
    CrossChainRouteFilter, CrossChainRoutePair, GetTokensMetadataRequest,
    GetTokensMetadataResponse, InputType, ListFiatCurrenciesResponse, ListFiatRatesResponse,
    Network, OptimizationMode, OptimizeLeavesRequest, OptimizeLeavesResponse,
    RegisterWebhookRequest, RegisterWebhookResponse, SignMessageRequest, SignMessageResponse,
    UnregisterWebhookRequest, UpdateUserSettingsRequest, UserSettings, Webhook,
    chain::RecommendedFees,
    error::SdkError,
    events::EventListener,
    issuer::TokenIssuer,
    models::{GetInfoRequest, GetInfoResponse, StableBalanceActiveLabel},
    persist::ObjectCacheRepository,
    utils::token::get_tokens_metadata_cached_or_query,
};

use super::{BreezSdk, helpers::get_deposit_address, parse_input};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Registers a listener to receive SDK events
    ///
    /// The SDK holds the listener until it is removed with
    /// `remove_event_listener` or until `disconnect` unregisters all
    /// listeners. A held listener that references the SDK instance keeps
    /// that instance alive.
    ///
    /// # Arguments
    ///
    /// * `listener` - An implementation of the `EventListener` trait
    ///
    /// # Returns
    ///
    /// A unique identifier for the listener, which can be used to remove it later
    pub async fn add_event_listener(&self, listener: Box<dyn EventListener>) -> String {
        self.event_emitter.add_external_listener(listener).await
    }

    /// Removes a previously registered event listener
    ///
    /// # Arguments
    ///
    /// * `id` - The listener ID returned from `add_event_listener`
    ///
    /// # Returns
    ///
    /// `true` if the listener was found and removed, `false` otherwise
    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.event_emitter.remove_external_listener(id).await
    }

    /// Stops the SDK's background tasks
    ///
    /// This method stops the background tasks started by the `start()` method.
    /// It should be called before your application terminates to ensure proper cleanup.
    ///
    /// It also unregisters all event listeners, so listeners that reference
    /// the SDK no longer keep it alive after this call.
    ///
    /// # Returns
    ///
    /// Result containing either success or an `SdkError` if the background task couldn't be stopped
    pub async fn disconnect(&self) -> Result<(), SdkError> {
        info!("Disconnecting Breez SDK");
        self.event_emitter.clear_external_listeners().await;
        if self.shutdown_sender.send(()).is_err() {
            // A `watch::Sender::send` error means every receiver has been
            // dropped, i.e. no background task is listening. This is the
            // expected steady state for a server-mode SDK
            // (`background_tasks_enabled = false`): there is nothing to
            // stop, so disconnecting is a successful no-op.
            debug!("No shutdown receivers; SDK has no background tasks to stop");
            return Ok(());
        }

        self.shutdown_sender.closed().await;
        info!("Breez SDK disconnected");
        Ok(())
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, SdkError> {
        parse_input(input, Some(self.external_input_parsers.clone())).await
    }

    /// Returns the available cross-chain routes.
    ///
    /// Use [`CrossChainRouteFilter::Send`] to get routes for sending from Spark
    /// (filtered by the parsed recipient address), or
    /// [`CrossChainRouteFilter::Receive`] to get routes for receiving into Spark
    /// (optionally filtered by a source contract address).
    pub async fn get_cross_chain_routes(
        &self,
        filter: &CrossChainRouteFilter,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError> {
        let mut all_routes = Vec::new();
        for svc in self.cross_chain_context.values() {
            match svc.get_routes(filter).await {
                Ok(routes) => all_routes.extend(routes),
                Err(e) => tracing::warn!("Cross-chain provider route fetch failed: {e}"),
            }
        }

        // Filter to USD-pegged destinations only.
        all_routes.retain(|r| crate::cross_chain::is_usd_stable_asset(&r.asset));

        all_routes.sort_by(|a, b| {
            a.asset
                .cmp(&b.asset)
                .then_with(|| a.chain.cmp(&b.chain))
                .then_with(|| a.provider.cmp(&b.provider))
        });
        Ok(all_routes)
    }

    /// Returns the balance of the wallet in satoshis
    #[allow(unused_variables)]
    pub async fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        self.runtime.get_info(self, request).await
    }

    /// List fiat currencies for which there is a known exchange rate,
    /// sorted by the canonical name of the currency.
    pub async fn list_fiat_currencies(&self) -> Result<ListFiatCurrenciesResponse, SdkError> {
        let currencies = self
            .fiat_service
            .fetch_fiat_currencies()
            .await?
            .into_iter()
            .map(From::from)
            .collect();
        Ok(ListFiatCurrenciesResponse { currencies })
    }

    /// List the latest rates of fiat currencies, sorted by name.
    pub async fn list_fiat_rates(&self) -> Result<ListFiatRatesResponse, SdkError> {
        let rates = self
            .fiat_service
            .fetch_fiat_rates()
            .await?
            .into_iter()
            .map(From::from)
            .collect();
        Ok(ListFiatRatesResponse { rates })
    }

    /// Get the recommended BTC fees based on the configured chain service.
    pub async fn recommended_fees(&self) -> Result<RecommendedFees, SdkError> {
        Ok(self.chain_service.recommended_fees().await?)
    }

    /// Returns the metadata for the given token identifiers.
    ///
    /// Results are not guaranteed to be in the same order as the input token identifiers.
    ///
    /// If the metadata is not found locally in cache, it will be queried from
    /// the Spark network and then cached.
    pub async fn get_tokens_metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> Result<GetTokensMetadataResponse, SdkError> {
        let metadata = get_tokens_metadata_cached_or_query(
            &self.spark_wallet,
            &ObjectCacheRepository::new(self.storage.clone()),
            &request
                .token_identifiers
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
        .await?;
        Ok(GetTokensMetadataResponse {
            tokens_metadata: metadata,
        })
    }

    /// Signs a message with the wallet's identity key. The message is SHA256
    /// hashed before signing. The returned signature will be hex encoded in
    /// DER format by default, or compact format if specified.
    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> Result<SignMessageResponse, SdkError> {
        use bitcoin::hex::DisplayHex;

        let pubkey = self.spark_wallet.get_identity_public_key().to_string();
        let signature = self.spark_wallet.sign_message(&request.message).await?;
        let signature_hex = if request.compact {
            signature.serialize_compact().to_lower_hex_string()
        } else {
            signature.serialize_der().to_lower_hex_string()
        };

        Ok(SignMessageResponse {
            pubkey,
            signature: signature_hex,
        })
    }

    /// Verifies a message signature against the provided public key. The message
    /// is SHA256 hashed before verification. The signature can be hex encoded
    /// in either DER or compact format.
    pub async fn check_message(
        &self,
        request: CheckMessageRequest,
    ) -> Result<CheckMessageResponse, SdkError> {
        let pubkey = PublicKey::from_str(&request.pubkey)
            .map_err(|_| SdkError::InvalidInput("Invalid public key".to_string()))?;
        let signature_bytes = hex::decode(&request.signature)
            .map_err(|_| SdkError::InvalidInput("Not a valid hex encoded signature".to_string()))?;
        let signature = Signature::from_der(&signature_bytes)
            .or_else(|_| Signature::from_compact(&signature_bytes))
            .map_err(|_| {
                SdkError::InvalidInput("Not a valid DER or compact encoded signature".to_string())
            })?;

        let is_valid = self
            .spark_wallet
            .verify_message(&request.message, &signature, &pubkey)
            .await
            .is_ok();
        Ok(CheckMessageResponse { is_valid })
    }

    /// Returns the user settings for the wallet.
    ///
    /// Some settings are fetched from the Spark network so network requests are performed.
    pub async fn get_user_settings(&self) -> Result<UserSettings, SdkError> {
        // Ensure spark private mode is initialized to avoid race conditions with the initialization task.
        self.maybe_ensure_spark_private_mode_initialized().await?;

        let spark_user_settings = self.spark_wallet.query_wallet_settings().await?;

        let stable_balance_active_label = match &self.stable_balance {
            Some(sb) => sb.get_active_label().await,
            None => None,
        };

        Ok(UserSettings {
            spark_private_mode_enabled: spark_user_settings.private_enabled,
            stable_balance_active_label,
        })
    }

    /// Updates the user settings for the wallet.
    ///
    /// Some settings are updated on the Spark network so network requests may be performed.
    pub async fn update_user_settings(
        &self,
        request: UpdateUserSettingsRequest,
    ) -> Result<(), SdkError> {
        if let Some(spark_private_mode_enabled) = request.spark_private_mode_enabled {
            self.spark_wallet
                .update_wallet_settings(spark_private_mode_enabled)
                .await?;
        }

        if let Some(active_label) = request.stable_balance_active_label {
            let sb = self
                .stable_balance
                .as_ref()
                .ok_or_else(|| SdkError::Generic("Stable balance is not configured".to_string()))?;
            let label = if let StableBalanceActiveLabel::Set { label } = active_label {
                Some(label)
            } else {
                None
            };
            sb.set_active_token(label).await?;
        }

        Ok(())
    }

    /// Returns an instance of the [`TokenIssuer`] for managing token issuance.
    pub fn get_token_issuer(&self) -> TokenIssuer {
        TokenIssuer::new(self.spark_wallet.clone(), self.storage.clone())
    }

    /// Manually drives leaf optimization, blocking until the requested work
    /// is done.
    ///
    /// With [`OptimizationMode::Full`] (the default) the call runs the entire
    /// optimization in a single invocation. With
    /// [`OptimizationMode::SingleRound`] it executes one round and returns —
    /// the caller drives the loop by inspecting the
    /// [`OptimizeLeavesResponse::outcome`] and calling again until
    /// `InProgress` no longer appears.
    ///
    /// Returns an error if another optimization run (auto or manual) is
    /// already in flight ([`SdkError::OptimizationAlreadyRunning`]), or if
    /// the SDK preempted this run to free leaves for a payment
    /// ([`SdkError::OptimizationCancelled`]).
    ///
    /// Manual runs do not emit events; events ([`SdkEvent::AutoOptimization`])
    /// are reserved for the background auto-optimizer.
    pub async fn optimize_leaves(
        &self,
        request: OptimizeLeavesRequest,
    ) -> Result<OptimizeLeavesResponse, SdkError> {
        let max_rounds = match request.mode {
            OptimizationMode::Full => None,
            OptimizationMode::SingleRound => Some(1),
        };
        let outcome = self.spark_wallet.optimize_leaves(max_rounds).await?.into();
        Ok(OptimizeLeavesResponse { outcome })
    }

    /// Registers a webhook to receive notifications for wallet events.
    ///
    /// When registered events occur (e.g., a Lightning payment is received),
    /// the Spark service provider will send an HTTP POST to the specified URL
    /// with a payload signed using HMAC-SHA256 with the provided secret.
    ///
    /// # Arguments
    ///
    /// * `request` - The webhook registration details including URL, secret, and event types
    ///
    /// # Returns
    ///
    /// A response containing the unique identifier of the registered webhook
    pub async fn register_webhook(
        &self,
        request: RegisterWebhookRequest,
    ) -> Result<RegisterWebhookResponse, SdkError> {
        let event_types = request.event_types.into_iter().map(Into::into).collect();
        let webhook_id = self
            .spark_wallet
            .register_wallet_webhook(&request.url, &request.secret, event_types)
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to register webhook: {e}")))?;
        Ok(RegisterWebhookResponse { webhook_id })
    }

    /// Unregisters a previously registered webhook.
    ///
    /// After unregistering, the Spark service provider will no longer send
    /// notifications to the webhook URL.
    ///
    /// # Arguments
    ///
    /// * `request` - The unregister request containing the webhook ID
    pub async fn unregister_webhook(
        &self,
        request: UnregisterWebhookRequest,
    ) -> Result<(), SdkError> {
        self.spark_wallet
            .delete_wallet_webhook(&request.webhook_id)
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to unregister webhook: {e}")))?;
        Ok(())
    }

    /// Lists all webhooks currently registered for this wallet.
    ///
    /// # Returns
    ///
    /// A list of registered webhooks with their IDs, URLs, and subscribed event types
    pub async fn list_webhooks(&self) -> Result<Vec<Webhook>, SdkError> {
        let webhooks = self
            .spark_wallet
            .list_wallet_webhooks()
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to list webhooks: {e}")))?;
        Ok(webhooks.into_iter().map(Into::into).collect())
    }

    /// Initiates a Bitcoin purchase flow via an external provider.
    ///
    /// Returns a URL the user should open to complete the purchase.
    /// The request variant determines the provider and its parameters:
    ///
    /// - [`BuyBitcoinRequest::Moonpay`]: Fiat-to-Bitcoin via on-chain deposit.
    /// - [`BuyBitcoinRequest::CashApp`]: Lightning invoice + `cash.app` deep link (mainnet only).
    pub async fn buy_bitcoin(
        &self,
        request: BuyBitcoinRequest,
    ) -> Result<BuyBitcoinResponse, SdkError> {
        let url = match request {
            BuyBitcoinRequest::Moonpay {
                locked_amount_sat,
                redirect_url,
            } => {
                let address = get_deposit_address(&self.spark_wallet, true).await?;
                self.buy_bitcoin_provider
                    .buy_bitcoin(address, locked_amount_sat, redirect_url)
                    .await
                    .map_err(|e| {
                        SdkError::Generic(format!("Failed to create buy bitcoin URL: {e}"))
                    })?
            }
            BuyBitcoinRequest::CashApp { amount_sats } => {
                if !matches!(self.config.network, Network::Mainnet) {
                    return Err(SdkError::Generic(
                        "CashApp is only available on mainnet".to_string(),
                    ));
                }
                if amount_sats == 0 {
                    return Err(SdkError::Generic(
                        "CashApp requires a non-zero amount".to_string(),
                    ));
                }
                let receive_response = self
                    .receive_bolt11_invoice(
                        "Buy Bitcoin via CashApp".to_string(),
                        Some(amount_sats),
                        None,
                        None,
                    )
                    .await?;
                CashAppProvider::build_url(&receive_response.payment_request)
            }
        };

        Ok(BuyBitcoinResponse { url })
    }
}
