use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use breez_sdk_common::{buy::cashapp::CashAppProvider, input::CrossChainAddressFamily};
use spark_wallet::MasterIdentityPublicKeyUpdate;
use std::str::FromStr;
use tracing::{debug, info};

use crate::{
    BuyBitcoinRequest, BuyBitcoinResponse, CheckMessageRequest, CheckMessageResponse,
    CrossChainProvider, CrossChainRouteFilter, CrossChainRoutePair, GetTokensMetadataRequest,
    GetTokensMetadataResponse, InputType, ListFiatCurrenciesResponse, ListFiatRatesResponse,
    Network, OptimizationMode, OptimizeLeavesRequest, OptimizeLeavesResponse,
    PreparePaymentLinkRequest, PreparePaymentLinkResponse, RegisterWebhookRequest,
    RegisterWebhookResponse, SignMessageRequest, SignMessageResponse, SourceChain,
    UnregisterWebhookRequest, UpdateUserSettingsRequest, UserSettings, Webhook,
    chain::RecommendedFees,
    cross_chain::{
        CrossChainProviderContext, convert_destination_amount_to_sats, fetch_btc_usd_rate,
    },
    error::SdkError,
    events::EventListener,
    issuer::TokenIssuer,
    models::{
        GetInfoRequest, GetInfoResponse, SparkMasterIdentityPublicKey, StableBalanceActiveLabel,
    },
    persist::ObjectCacheRepository,
    utils::token::get_tokens_metadata_cached_or_query,
};

use super::payments::validation::{
    known_token_contracts, resolve_direct_overpay_amount, resolve_slippage_bps,
    resolve_target_overpay_bps, validate_address_family_against_route, validate_amount,
    validate_recipient_not_contract_address,
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
    /// (filtered by the parsed recipient address),
    /// [`CrossChainRouteFilter::PaymentLink`] for routes fundable by an external
    /// fiat rail (`prepare_payment_link`), or
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
                        None,
                    )
                    .await?;
                CashAppProvider::build_url(&receive_response.payment_request)
            }
        };

        Ok(BuyBitcoinResponse { url })
    }

    /// Prepare a payment link that sends USDC/USDT to an external-chain
    /// recipient, funded by Cash App over Lightning.
    ///
    /// Creates a cross-chain order and returns a `cash.app` deep link the payer
    /// opens. Once paid, the provider delivers the stablecoin to `address`. No
    /// funds move through the Spark wallet. Only available on mainnet.
    pub async fn prepare_payment_link(
        &self,
        request: PreparePaymentLinkRequest,
    ) -> Result<PreparePaymentLinkResponse, SdkError> {
        if !matches!(self.config.network, Network::Mainnet) {
            return Err(SdkError::Generic("Only available on mainnet".to_string()));
        }
        validate_amount(Some(request.amount))?;

        let PreparePaymentLinkRequest {
            address,
            route,
            amount,
            fee_policy,
            max_slippage_bps,
        } = request;

        // Payment links are Orchestra-only. A Boltz reverse swap needs this
        // wallet online to claim before the payer's HTLC settles, so it can't
        // back a link someone else pays later. Orchestra orders are submitless
        // and deliver without the SDK.
        if !matches!(route.provider, CrossChainProvider::Orchestra) {
            return Err(SdkError::InvalidInput(
                "Payment links are only supported on Orchestra routes".to_string(),
            ));
        }

        // Cash App funds the deposit over Lightning. Fail fast before quoting if
        // the route can't be funded that way. Rejecting an empty
        // `supported_source_chains` stops a hand-built route from reaching
        // `prepare` and committing provider state it can't fund.
        if !route_supports_source_chain(&route, SourceChain::Lightning) {
            return Err(SdkError::InvalidInput(
                "The selected route can't be funded over Lightning".to_string(),
            ));
        }

        // Validate the recipient before creating the order, using the same guards
        // as the send path.
        let InputType::CrossChainAddress(recipient) = self.parse(&address).await? else {
            return Err(SdkError::InvalidInput(
                "Recipient must be a cross-chain (EVM/Solana/Tron) address".to_string(),
            ));
        };
        let recipient_family: CrossChainAddressFamily = recipient.address_family.into();
        validate_address_family_against_route(recipient_family, &route)?;
        let known_contracts =
            known_token_contracts(self, &recipient.address, recipient_family, &route).await;
        validate_recipient_not_contract_address(
            &recipient.address,
            recipient_family,
            &known_contracts,
        )?;

        let service = self.cross_chain_context.get(route.provider)?;
        let slippage_bps = resolve_slippage_bps(
            max_slippage_bps,
            self.config
                .cross_chain_config
                .as_ref()
                .and_then(|c| c.default_slippage_bps),
        )?;

        // `amount` is in the destination asset's base units (the route's
        // `decimals`); for these USD-pegged stablecoins it equals the USD value
        // at parity. Convert it to the sats the provider's `prepare` expects
        // (for both fee modes) via the live rate.
        let btc_usd = fetch_btc_usd_rate(self.cross_chain_context.fiat_service().as_ref()).await?;
        let source_sats =
            convert_destination_amount_to_sats(amount, btc_usd, route.decimals.into())?;
        if source_sats == 0 {
            return Err(SdkError::InvalidInput(
                "Amount is too small to fund over Lightning".to_string(),
            ));
        }

        // Match the send path: on `FeesExcluded` pad the source so the recipient
        // lands at or above target despite provider slippage. The pad comes from
        // config (no per-request override on a payment link).
        let fee_policy = fee_policy.unwrap_or_default();
        let overpay_bps = resolve_target_overpay_bps(
            None,
            self.config
                .cross_chain_config
                .as_ref()
                .and_then(|c| c.default_target_overpay_bps),
        )?;
        let source_sats = resolve_direct_overpay_amount(source_sats, fee_policy, overpay_bps);

        let prepared = service
            .prepare(
                &recipient.address,
                &route,
                source_sats,
                Some(SourceChain::Lightning),
                None,
                slippage_bps,
                fee_policy.into(),
            )
            .await?;

        let amount_sats = u64::try_from(prepared.amount_in).map_err(|_| {
            SdkError::Generic(format!(
                "Deposit amount {} sats exceeds u64::MAX",
                prepared.amount_in
            ))
        })?;

        // The provider returns the BOLT11 the external payer funds in its
        // context. We build the `cash.app` deep link ourselves and never call
        // the send stage: the external payer funds the provider's deposit
        // directly, so there is no Spark-side transfer to make and the provider
        // delivers autonomously once the payer settles.
        let deposit_target = deposit_target(&prepared.provider_context);

        // Verify the provider's target is a BOLT11 for the quoted amount before
        // sending the payer to it.
        let parsed_target = self.parse(&deposit_target).await;
        check_deposit_target(&parsed_target, amount_sats)?;

        let url = CashAppProvider::build_url(&deposit_target);

        Ok(prepare_payment_link_response(prepared, url, amount_sats))
    }
}

/// Whether `route` advertises `required` as a fundable source chain. An empty
/// `supported_source_chains` (e.g. a hand-built route) matches nothing and is
/// rejected.
fn route_supports_source_chain(route: &CrossChainRoutePair, required: SourceChain) -> bool {
    route.supported_source_chains.contains(&required)
}

/// Verifies the provider's deposit `target` is a BOLT11 invoice requesting
/// exactly `expected_sats`, the Lightning payment request Cash App funds. A
/// wrong type, wrong amount, or unparseable target indicates a provider bug.
fn check_deposit_target(
    parsed_target: &Result<InputType, SdkError>,
    expected_sats: u64,
) -> Result<(), SdkError> {
    let Ok(InputType::Bolt11Invoice(details)) = parsed_target else {
        return Err(SdkError::Generic(
            "The provider returned a deposit target that is not a valid Lightning \
             payment request"
                .to_string(),
        ));
    };
    // The invoice must request the quoted deposit: a mismatch would have the
    // payer fund the wrong amount.
    if details.amount_msat.map(u128::from) != Some(u128::from(expected_sats) * 1000) {
        return Err(SdkError::Generic(format!(
            "The provider's deposit invoice does not request the quoted \
             {expected_sats} sats"
        )));
    }
    Ok(())
}

/// The BOLT11 an external payer funds, read from the provider context
/// (Orchestra `deposit_address` or Boltz `invoice`).
fn deposit_target(context: &CrossChainProviderContext) -> String {
    match context {
        CrossChainProviderContext::Orchestra {
            deposit_address, ..
        } => deposit_address.clone(),
        CrossChainProviderContext::Boltz { invoice, .. } => invoice.clone(),
    }
}

/// Maps a [`crate::cross_chain::CrossChainPrepared`] + funding `url` to a
/// [`PreparePaymentLinkResponse`].
///
/// `estimated_out` is in the destination `asset`'s base units. The fee mirrors
/// the cross-chain send response: `service_fee_amount` in `service_fee_asset`
/// base units, where `None` means sats (Boltz denominates its fee in sats;
/// Orchestra in the stablecoin).
fn prepare_payment_link_response(
    prepared: crate::cross_chain::CrossChainPrepared,
    url: String,
    amount_sats: u64,
) -> PreparePaymentLinkResponse {
    PreparePaymentLinkResponse {
        url,
        amount_sats,
        estimated_out: prepared.estimated_out,
        asset: prepared.pair.asset,
        service_fee_amount: prepared.service_fee_amount,
        service_fee_asset: prepared.service_fee_asset,
        expires_at: normalize_expires_at(&prepared.expires_at),
    }
}

/// Normalizes a provider `expires_at` to RFC3339. Orchestra already returns
/// RFC3339; Boltz returns a unix-seconds string, which we convert so callers
/// see a single format.
fn normalize_expires_at(raw: &str) -> String {
    if let Ok(secs) = raw.parse::<i64>()
        && let Some(dt) = chrono::DateTime::from_timestamp(secs, 0)
    {
        return dt.to_rfc3339();
    }
    raw.to_string()
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
    use super::{
        CashAppProvider, SdkError, deposit_target, parse_compressed_public_key,
        prepare_payment_link_response, route_supports_source_chain,
    };
    use crate::cross_chain::{CrossChainPrepared, CrossChainProviderContext};
    use crate::{CrossChainFeeMode, CrossChainProvider, CrossChainRoutePair, SourceChain};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn orchestra_context() -> CrossChainProviderContext {
        CrossChainProviderContext::Orchestra {
            quote_id: "q1".to_string(),
            deposit_address: "lnbc100u1pxyz".to_string(),
            deposit_amount: 5000,
        }
    }

    fn route_with_source_chains(chains: Vec<SourceChain>) -> CrossChainRoutePair {
        CrossChainRoutePair {
            provider: CrossChainProvider::Orchestra,
            chain: "base".to_string(),
            chain_id: Some("8453".to_string()),
            asset: "USDC".to_string(),
            contract_address: None,
            decimals: 6,
            exact_out_eligible: false,
            supported_sources: vec![],
            supported_source_chains: chains,
        }
    }

    #[test_all]
    fn route_supports_source_chain_checks_membership() {
        let lightning_only = route_with_source_chains(vec![SourceChain::Lightning]);
        // Cash App (Lightning) is supported; MoonPay (Bitcoin) is not.
        assert!(route_supports_source_chain(
            &lightning_only,
            SourceChain::Lightning
        ));
        assert!(!route_supports_source_chain(
            &lightning_only,
            SourceChain::Bitcoin
        ));

        let both = route_with_source_chains(vec![SourceChain::Lightning, SourceChain::Bitcoin]);
        assert!(route_supports_source_chain(&both, SourceChain::Bitcoin));

        // An empty list matches nothing: a route must advertise its rails, so a
        // hand-built route can't slip a bad rail through to `prepare`.
        let empty = route_with_source_chains(vec![]);
        assert!(!route_supports_source_chain(&empty, SourceChain::Bitcoin));
    }

    fn prepared(provider_context: CrossChainProviderContext) -> CrossChainPrepared {
        CrossChainPrepared {
            amount_in: 5000,
            asset_amount_in: 6_500_000,
            estimated_out: 6_450_000,
            fee_amount: 50_000,
            service_fee_amount: 47_684,
            service_fee_asset: Some("USDC".to_string()),
            source_transfer_fee_sats: 0,
            fee_mode: CrossChainFeeMode::FeesExcluded,
            expires_at: "2026-07-25T08:09:26.770Z".to_string(),
            pair: CrossChainRoutePair {
                provider: CrossChainProvider::Orchestra,
                chain: "base".to_string(),
                chain_id: Some("8453".to_string()),
                asset: "USDC".to_string(),
                contract_address: None,
                decimals: 6,
                exact_out_eligible: false,
                supported_sources: vec![],
                supported_source_chains: vec![],
            },
            recipient_address: "0xabc".to_string(),
            token_identifier: None,
            provider_context,
        }
    }

    #[test_all]
    fn deposit_target_reads_orchestra_address_and_boltz_invoice() {
        assert_eq!(deposit_target(&orchestra_context()), "lnbc100u1pxyz");
        assert_eq!(
            deposit_target(&CrossChainProviderContext::Boltz {
                swap_id: "s1".to_string(),
                invoice: "lnbc200u1pboltz".to_string(),
                invoice_amount_sats: 5000,
                max_slippage_bps: 100,
            }),
            "lnbc200u1pboltz"
        );
    }

    #[test_all]
    fn cashapp_url_built_from_deposit_target() {
        let url = CashAppProvider::build_url(&deposit_target(&orchestra_context()));
        assert_eq!(url, "https://cash.app/launch/lightning/lnbc100u1pxyz");
    }

    #[test_all]
    fn orchestra_response_maps_stablecoin_fee() {
        let resp = prepare_payment_link_response(
            prepared(orchestra_context()),
            "https://x".to_string(),
            5000,
        );
        assert_eq!(resp.url, "https://x");
        assert_eq!(resp.amount_sats, 5000);
        assert_eq!(resp.estimated_out, 6_450_000);
        assert_eq!(resp.asset, "USDC");
        // Orchestra denominates its service fee in the stablecoin.
        assert_eq!(resp.service_fee_amount, 47_684);
        assert_eq!(resp.service_fee_asset.as_deref(), Some("USDC"));
    }

    #[test_all]
    fn boltz_response_reports_native_sats_fee() {
        let mut prepared = prepared(CrossChainProviderContext::Boltz {
            swap_id: "s1".to_string(),
            invoice: "lnbc200u1pboltz".to_string(),
            invoice_amount_sats: 5000,
            max_slippage_bps: 100,
        });
        // Boltz denominates its service fee in sats (service_fee_asset = None).
        prepared.service_fee_amount = 17;
        prepared.service_fee_asset = None;
        let resp = prepare_payment_link_response(prepared, "https://x".to_string(), 5000);
        assert_eq!(resp.asset, "USDC");
        assert_eq!(resp.service_fee_amount, 17);
        assert_eq!(resp.service_fee_asset, None);
    }

    #[test_all]
    fn normalize_expires_at_converts_unix_and_passes_rfc3339() {
        // Boltz-style unix seconds get converted to a parseable RFC3339 string.
        let converted = super::normalize_expires_at("1784895588");
        assert_ne!(converted, "1784895588");
        assert!(chrono::DateTime::parse_from_rfc3339(&converted).is_ok());
        // Orchestra-style RFC3339 passes through unchanged.
        let iso = "2026-07-25T08:09:26.770Z";
        assert_eq!(super::normalize_expires_at(iso), iso);
    }

    const COMPRESSED: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const UNCOMPRESSED: &str = "0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f8179\
        8483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8";

    #[test]
    fn parse_compressed_public_key_round_trips() {
        let key = parse_compressed_public_key(COMPRESSED).unwrap();
        assert_eq!(key.to_string(), COMPRESSED);
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
