use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use spark_wallet::MasterIdentityPublicKeyUpdate;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info};

use crate::{
    BridgeFromCashAppRequest, BridgeFromCashAppResponse, BridgeToCashAppRequest,
    BridgeToCashAppResponse, CheckMessageRequest, CheckMessageResponse, CrossChainRouteFilter,
    CrossChainRoutePair, GetTokensMetadataRequest, GetTokensMetadataResponse, InputType,
    ListFiatCurrenciesResponse, ListFiatRatesResponse, Network, OptimizationMode,
    OptimizeLeavesRequest, OptimizeLeavesResponse, PreparePaymentLinkRequest,
    PreparePaymentLinkResponse, RegisterWebhookRequest, RegisterWebhookResponse,
    SignMessageRequest, SignMessageResponse, UnregisterWebhookRequest, UpdateUserSettingsRequest,
    UserSettings, Webhook,
    chain::RecommendedFees,
    error::SdkError,
    events::EventListener,
    issuer::TokenIssuer,
    models::{
        GetInfoRequest, GetInfoResponse, SparkMasterIdentityPublicKey, StableBalanceActiveLabel,
    },
    persist::ObjectCacheRepository,
    utils::token::get_tokens_metadata_cached_or_query,
};

use super::BreezSdk;
use super::bridging::BridgingClient;

#[macros::instance_span]
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
        Ok(self.input_parser.parse(input).await?.into())
    }

    /// Returns the available cross-chain routes.
    ///
    /// Use [`CrossChainRouteFilter::Send`] to get routes for sending to
    /// another chain (filtered by the parsed recipient address), or
    /// [`CrossChainRouteFilter::Receive`] to get routes for receiving from
    /// another chain (optionally filtered by a source contract address). By
    /// default both list the routes paid from or delivered to the Spark
    /// balance. Set their `delivery_method` to `Lightning` to list the routes an
    /// external payer funds over Lightning, or that pay out to a Lightning
    /// address.
    ///
    /// Fails with a `CrossChainDisabled` error when cross-chain payments are not
    /// enabled in the config. An empty list means no route matches the filter.
    pub async fn get_cross_chain_routes(
        &self,
        filter: &CrossChainRouteFilter,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError> {
        self.cross_chain_context.list_routes(filter).await
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
    ///
    /// Messages starting with `breez-lnurl:` or `kind: rewards_endorsement` are
    /// refused: they are reserved for the SDK's own requests to the Lightning
    /// address server and to Flashnet's rewards service.
    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> Result<SignMessageResponse, SdkError> {
        use bitcoin::hex::DisplayHex;

        reject_reserved_namespace(&request.message)?;

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
            spark_master_identity_public_key: spark_user_settings
                .master_identity_public_key
                .map(|key| key.to_string()),
        })
    }

    /// Updates the user settings for the wallet.
    ///
    /// Some settings are updated on the Spark network so network requests may be performed.
    pub async fn update_user_settings(
        &self,
        request: UpdateUserSettingsRequest,
    ) -> Result<(), SdkError> {
        let master_identity_public_key = request
            .spark_master_identity_public_key
            .map(|update| match update {
                SparkMasterIdentityPublicKey::Set { public_key } => {
                    parse_compressed_public_key(&public_key).map(MasterIdentityPublicKeyUpdate::Set)
                }
                SparkMasterIdentityPublicKey::Unset => Ok(MasterIdentityPublicKeyUpdate::Clear),
            })
            .transpose()?;

        self.spark_wallet
            .update_wallet_settings(
                request.spark_private_mode_enabled,
                master_identity_public_key,
            )
            .await?;

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
        let result = self.spark_wallet.optimize_leaves(max_rounds).await;
        Ok(OptimizeLeavesResponse {
            outcome: result?.into(),
        })
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

    /// **Deprecated.** Use `bridge_from_cash_app` on the client from
    /// [`get_bridging_client`](Self::get_bridging_client).
    pub async fn prepare_payment_link(
        &self,
        request: PreparePaymentLinkRequest,
    ) -> Result<PreparePaymentLinkResponse, SdkError> {
        self.bridge_from_cash_app(request.into())
            .await
            .map(Into::into)
    }

    /// **Deprecated.** Use `bridge_from_cash_app` on the client from
    /// [`get_bridging_client`](Self::get_bridging_client), whose bridges can be
    /// looked up with `get_bridge_status`.
    ///
    /// Bridge from Cash App to USDC/USDT on an external chain.
    ///
    /// Creates a cross-chain order and returns a `cash.app` deep link the payer
    /// opens. Once paid over Lightning, the provider delivers the stablecoin to
    /// `address`, or refunds a failure before the swap to this wallet's
    /// Lightning address, when one is registered. Nothing is tracked. Only
    /// available on mainnet.
    pub async fn bridge_from_cash_app(
        &self,
        request: BridgeFromCashAppRequest,
    ) -> Result<BridgeFromCashAppResponse, SdkError> {
        if !matches!(self.config.network, Network::Mainnet) {
            return Err(SdkError::Generic("Only available on mainnet".to_string()));
        }
        self.cross_chain_context.ensure_enabled()?;
        let bridging_client = self.get_bridging_client()?;
        let refund_address = self.wallet_refund_address().await;
        bridging_client
            .legacy_bridge_from_cash_app(request, refund_address.as_deref())
            .await
    }

    /// **Deprecated.** Use `bridge_to_cash_app` on the client from
    /// [`get_bridging_client`](Self::get_bridging_client), whose bridges can be
    /// looked up with `get_bridge_status`.
    ///
    /// Bridge USDC/USDT from an external chain to a Cash App user.
    ///
    /// Returns the deposit the payer makes on the source chain. Once it lands,
    /// the cross-chain provider pays the recipient in Bitcoin over Lightning,
    /// or refunds `refund_address` if it can't. No funds move through the
    /// Spark wallet, and nothing is tracked. Only available on mainnet.
    pub async fn bridge_to_cash_app(
        &self,
        request: BridgeToCashAppRequest,
    ) -> Result<BridgeToCashAppResponse, SdkError> {
        if !matches!(self.config.network, Network::Mainnet) {
            return Err(SdkError::Generic("Only available on mainnet".to_string()));
        }
        self.cross_chain_context.ensure_enabled()?;
        self.get_bridging_client()?
            .legacy_bridge_to_cash_app(request)
            .await
    }

    /// Returns a client for bridges: orders in which a payer on another chain
    /// or on Lightning pays a recipient, with no funds moving through the
    /// SDK. It shares this SDK's API key, and its cross-chain settings when
    /// set. It doesn't need cross-chain payments enabled, so it also works in
    /// server mode. Only available on mainnet.
    pub fn get_bridging_client(&self) -> Result<Arc<BridgingClient>, SdkError> {
        let quoter = self
            .cross_chain_context
            .orchestra_quoter()
            .ok_or_else(|| SdkError::Generic("Only available on mainnet".to_string()))?;
        Ok(Arc::new(BridgingClient::new(
            self.config.cross_chain_config.clone(),
            Arc::clone(quoter),
            Arc::clone(self.cross_chain_context.fiat_service()),
            Arc::clone(&self.input_parser),
        )))
    }
}

/// Message prefixes reserved for the SDK's own signed requests, each with what
/// it is reserved for.
const RESERVED_PREFIXES: [(&str, &str); 2] = [
    (
        lnurl_models::signed_message::RESERVED_NAMESPACE,
        "Lightning address requests",
    ),
    (
        flashnet::rewards::ENDORSEMENT_PAYLOAD_PREFIX,
        "Flashnet rewards endorsements",
    ),
];

/// Refuses a message starting with a prefix reserved for the SDK's own
/// requests. Matching is ASCII case-insensitive.
fn reject_reserved_namespace(message: &str) -> Result<(), SdkError> {
    for (prefix, purpose) in RESERVED_PREFIXES {
        let reserved = message
            .as_bytes()
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix.as_bytes()));
        if reserved {
            return Err(SdkError::InvalidInput(format!(
                "messages starting with '{prefix}' are reserved for {purpose} and cannot be signed here"
            )));
        }
    }
    Ok(())
}

/// Parses a 33-byte compressed public key from hex.
///
/// Rejects the uncompressed encoding, which `PublicKey::from_str` also accepts:
/// Spark serializes identity keys compressed, so accepting it would make the
/// key read back differ from the one that was set.
fn parse_compressed_public_key(hex_encoded: &str) -> Result<PublicKey, SdkError> {
    let invalid = || SdkError::InvalidInput("Invalid master identity public key".to_string());
    let bytes: [u8; 33] = hex::decode(hex_encoded)
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())?;
    PublicKey::from_slice(&bytes).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::{SdkError, parse_compressed_public_key, reject_reserved_namespace};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const COMPRESSED: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const UNCOMPRESSED: &str = "0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f8179\
        8483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8";

    #[test]
    fn parse_compressed_public_key_round_trips() {
        let key = parse_compressed_public_key(COMPRESSED).unwrap();
        assert_eq!(key.to_string(), COMPRESSED);
    }

    #[test]
    fn reject_reserved_namespace_refuses_lnurl_messages() {
        let messages = [
            lnurl_models::signed_message::register("lnurl.example.com", "alice", Some("d"), 1),
            lnurl_models::signed_message::unregister("lnurl.example.com", "alice", 1),
            lnurl_models::signed_message::recover("lnurl.example.com", COMPRESSED, 1),
            lnurl_models::signed_message::metadata("lnurl.example.com", COMPRESSED, 1),
            "BREEZ-LNURL:v2\nregister\n...".to_string(),
            "breez-lnurl:".to_string(),
            "breez-lnurl:v3 whatever comes next".to_string(),
        ];
        for message in messages {
            assert!(
                matches!(
                    reject_reserved_namespace(&message),
                    Err(SdkError::InvalidInput(_))
                ),
                "expected {message:?} to be refused"
            );
        }
    }

    #[test]
    fn reject_reserved_namespace_refuses_rewards_endorsements() {
        for message in [
            "kind: rewards_endorsement\nversion: 1\naction: upsert\n...",
            "KIND: REWARDS_ENDORSEMENT\naction: delete",
            "kind: rewards_endorsement",
        ] {
            assert!(
                matches!(
                    reject_reserved_namespace(message),
                    Err(SdkError::InvalidInput(_))
                ),
                "expected {message:?} to be refused"
            );
        }
    }

    #[test]
    fn reject_reserved_namespace_allows_everything_else() {
        for message in [
            "",
            "hello",
            "breez-lnur",
            "breez lnurl:v2",
            // Only a leading namespace authorizes anything.
            "please sign breez-lnurl:v2\nregister",
            "please sign kind: rewards_endorsement",
            "kind: rewards",
            // A multi-byte first character must not panic the prefix check.
            "🥖",
        ] {
            assert!(
                reject_reserved_namespace(message).is_ok(),
                "expected {message:?} to be allowed"
            );
        }
    }

    #[test]
    fn parse_compressed_public_key_rejects_invalid() {
        for input in ["", "not-a-public-key", UNCOMPRESSED, &COMPRESSED[..64]] {
            assert!(
                matches!(
                    parse_compressed_public_key(input),
                    Err(SdkError::InvalidInput(_))
                ),
                "expected {input} to be rejected"
            );
        }
    }
}
