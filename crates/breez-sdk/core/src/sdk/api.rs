use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use std::str::FromStr;
use tracing::{error, info};

use crate::{
    AuthAction, BuyBitcoinRequest, BuyBitcoinResponse, CheckMessageRequest, CheckMessageResponse,
    GetTokensMetadataRequest, GetTokensMetadataResponse, InputType, ListFiatCurrenciesResponse,
    ListFiatRatesResponse, LnurlCallbackStatus, LnurlPayRequest, LnurlWithdrawRequest,
    OptimizationProgress, ParsedAction, PrepareLnurlPayRequest, PrepareSendActionResponse,
    PrepareSendPaymentRequest, ReceiveAction, SendAction, SendActionResponse, SendOptions,
    SendPaymentOptions, SendPaymentRequest, SignMessageRequest, SignMessageResponse,
    UpdateUserSettingsRequest, UserSettings,
    chain::RecommendedFees,
    error::SdkError,
    events::EventListener,
    issuer::TokenIssuer,
    models::{GetInfoRequest, GetInfoResponse, LnurlWithdrawResponse},
    persist::ObjectCacheRepository,
    utils::token::get_tokens_metadata_cached_or_query,
};

use super::{BreezSdk, helpers::get_or_create_deposit_address, parse_input};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Registers a listener to receive SDK events
    ///
    /// # Arguments
    ///
    /// * `listener` - An implementation of the `EventListener` trait
    ///
    /// # Returns
    ///
    /// A unique identifier for the listener, which can be used to remove it later
    pub async fn add_event_listener(&self, listener: Box<dyn EventListener>) -> String {
        self.event_emitter.add_listener(listener).await
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
        self.event_emitter.remove_listener(id).await
    }

    /// Stops the SDK's background tasks
    ///
    /// This method stops the background tasks started by the `start()` method.
    /// It should be called before your application terminates to ensure proper cleanup.
    ///
    /// # Returns
    ///
    /// Result containing either success or an `SdkError` if the background task couldn't be stopped
    pub async fn disconnect(&self) -> Result<(), SdkError> {
        info!("Disconnecting Breez SDK");
        self.shutdown_sender
            .send(())
            .map_err(|_| SdkError::Generic("Failed to send shutdown signal".to_string()))?;

        self.shutdown_sender.closed().await;
        info!("Breez SDK disconnected");
        Ok(())
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, SdkError> {
        parse_input(input, Some(self.external_input_parsers.clone())).await
    }

    /// Returns the balance of the wallet in satoshis
    #[allow(unused_variables)]
    pub async fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        if request.ensure_synced.unwrap_or_default() {
            self.initial_synced_watcher
                .clone()
                .changed()
                .await
                .map_err(|_| {
                    SdkError::Generic("Failed to receive initial synced signal".to_string())
                })?;
        }
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let account_info = object_repository
            .fetch_account_info()
            .await?
            .unwrap_or_default();
        Ok(GetInfoResponse {
            identity_pubkey: self.spark_wallet.get_identity_public_key().to_string(),
            balance_sats: account_info.balance_sats,
            token_balances: account_info.token_balances,
        })
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
        self.ensure_spark_private_mode_initialized().await?;

        let spark_user_settings = self.spark_wallet.query_wallet_settings().await?;

        // We may in the future have user settings that are stored locally and synced using real-time sync.

        Ok(UserSettings {
            spark_private_mode_enabled: spark_user_settings.private_enabled,
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

            // Reregister the lightning address if spark private mode changed.
            let lightning_address = match self.get_lightning_address().await {
                Ok(lightning_address) => lightning_address,
                Err(e) => {
                    error!("Failed to get lightning address during user settings update: {e:?}");
                    return Ok(());
                }
            };
            let Some(lightning_address) = lightning_address else {
                return Ok(());
            };
            if let Err(e) = self
                .register_lightning_address_internal(crate::RegisterLightningAddressRequest {
                    username: lightning_address.username,
                    description: Some(lightning_address.description),
                })
                .await
            {
                error!("Failed to reregister lightning address during user settings update: {e:?}");
            }
        }
        Ok(())
    }

    /// Returns an instance of the [`TokenIssuer`] for managing token issuance.
    pub fn get_token_issuer(&self) -> TokenIssuer {
        TokenIssuer::new(self.spark_wallet.clone(), self.storage.clone())
    }

    /// Starts leaf optimization in the background.
    ///
    /// This method spawns the optimization work in a background task and returns
    /// immediately. Progress is reported via events.
    /// If optimization is already running, no new task will be started.
    pub fn start_leaf_optimization(&self) {
        self.spark_wallet.start_leaf_optimization();
    }

    /// Cancels the ongoing leaf optimization.
    ///
    /// This method cancels the ongoing optimization and waits for it to fully stop.
    /// The current round will complete before stopping. This method blocks
    /// until the optimization has fully stopped and leaves reserved for optimization
    /// are available again.
    ///
    /// If no optimization is running, this method returns immediately.
    pub async fn cancel_leaf_optimization(&self) -> Result<(), SdkError> {
        self.spark_wallet.cancel_leaf_optimization().await?;
        Ok(())
    }

    /// Returns the current optimization progress snapshot.
    pub fn get_leaf_optimization_progress(&self) -> OptimizationProgress {
        self.spark_wallet.get_leaf_optimization_progress().into()
    }

    /// Initiates a Bitcoin purchase flow via an external provider (`MoonPay`).
    ///
    /// This method generates a URL that the user can open in a browser to complete
    /// the Bitcoin purchase. The purchased Bitcoin will be sent to an automatically
    /// generated deposit address.
    ///
    /// # Arguments
    ///
    /// * `request` - The purchase request containing optional amount and redirect URL
    ///
    /// # Returns
    ///
    /// A response containing the URL to open in a browser to complete the purchase
    pub async fn buy_bitcoin(
        &self,
        request: BuyBitcoinRequest,
    ) -> Result<BuyBitcoinResponse, SdkError> {
        let address =
            get_or_create_deposit_address(&self.spark_wallet, self.storage.clone(), true).await?;

        let url = self
            .buy_bitcoin_provider
            .buy_bitcoin(address, request.locked_amount_sat, request.redirect_url)
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to create buy bitcoin URL: {e}")))?;

        Ok(BuyBitcoinResponse { url })
    }
}

// Action-based API methods.
//
// These use `ParsedAction` / `SendAction` / etc. which contain recursive enums
// and trait objects that aren't directly UniFFI-exportable. They're available
// from Rust, WASM, and Flutter via their respective wrappers.
impl BreezSdk {
    /// Parses an input string and returns a structured [`ParsedAction`].
    ///
    /// This is a higher-level alternative to [`parse()`](BreezSdk::parse) that
    /// categorizes the result into Send, Receive, Authenticate, or Multi actions.
    pub async fn parse_action(&self, input: &str) -> Result<ParsedAction, SdkError> {
        let input_type = parse_input(input, Some(self.external_input_parsers.clone())).await?;
        Ok(ParsedAction::from(input_type))
    }

    /// Prepares a send payment from a [`SendAction`].
    ///
    /// This is a convenience wrapper that routes to the appropriate preparation method
    /// based on the action type:
    /// - For LNURL-pay and Lightning Address actions, routes to
    ///   [`prepare_lnurl_pay()`](BreezSdk::prepare_lnurl_pay)
    /// - For all other send actions, routes to
    ///   [`prepare_send_payment()`](BreezSdk::prepare_send_payment)
    ///
    /// Pass the result to [`send()`](BreezSdk::send) to execute the payment.
    ///
    /// # Options
    ///
    /// Use [`SendOptions`] to customize LNURL-pay specific parameters like `comment`,
    /// `validate_success_action_url`, `conversion_options`, and `fee_policy`.
    /// For non-LNURL payments, only `conversion_options` and `fee_policy` are used.
    pub async fn prepare_send(
        &self,
        action: &SendAction,
        amount: Option<u128>,
        token_identifier: Option<String>,
        options: Option<SendOptions>,
    ) -> Result<PrepareSendActionResponse, SdkError> {
        let options = options.unwrap_or_default();

        match action {
            SendAction::LnurlPay { pay_details } => {
                let amount_sats: u64 = amount
                    .ok_or(SdkError::InvalidInput(
                        "Amount is required for LNURL-pay".to_string(),
                    ))?
                    .try_into()
                    .map_err(|_| SdkError::InvalidInput("Amount overflow".to_string()))?;

                let response = self
                    .prepare_lnurl_pay(PrepareLnurlPayRequest {
                        amount_sats,
                        pay_request: pay_details.clone(),
                        comment: options.comment,
                        validate_success_action_url: options.validate_success_action_url,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                Ok(PrepareSendActionResponse::LnurlPay(Box::new(response)))
            }
            SendAction::LightningAddress { address_details } => {
                let amount_sats: u64 = amount
                    .ok_or(SdkError::InvalidInput(
                        "Amount is required for Lightning Address".to_string(),
                    ))?
                    .try_into()
                    .map_err(|_| SdkError::InvalidInput("Amount overflow".to_string()))?;

                let response = self
                    .prepare_lnurl_pay(PrepareLnurlPayRequest {
                        amount_sats,
                        pay_request: address_details.pay_request.clone(),
                        comment: options.comment,
                        validate_success_action_url: options.validate_success_action_url,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                Ok(PrepareSendActionResponse::LnurlPay(Box::new(response)))
            }
            _ => {
                let response = self
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request: action.payment_request(),
                        amount,
                        token_identifier,
                        conversion_options: options.conversion_options,
                        fee_policy: options.fee_policy,
                    })
                    .await?;

                Ok(PrepareSendActionResponse::Payment(Box::new(response)))
            }
        }
    }

    /// Sends a payment using a prepared response from [`prepare_send()`](BreezSdk::prepare_send).
    ///
    /// This is a convenience wrapper that routes to the appropriate send method based
    /// on the preparation result:
    /// - [`PrepareSendActionResponse::Payment`] routes to
    ///   [`send_payment()`](BreezSdk::send_payment)
    /// - [`PrepareSendActionResponse::LnurlPay`] routes to
    ///   [`lnurl_pay()`](BreezSdk::lnurl_pay)
    pub async fn send(
        &self,
        prepare_response: PrepareSendActionResponse,
        options: Option<SendPaymentOptions>,
        idempotency_key: Option<String>,
    ) -> Result<SendActionResponse, SdkError> {
        match prepare_response {
            PrepareSendActionResponse::Payment(prepare) => {
                let response = self
                    .send_payment(SendPaymentRequest {
                        prepare_response: *prepare,
                        options,
                        idempotency_key,
                    })
                    .await?;
                Ok(SendActionResponse::Payment(response))
            }
            PrepareSendActionResponse::LnurlPay(prepare) => {
                let response = self
                    .lnurl_pay(LnurlPayRequest {
                        prepare_response: *prepare,
                        idempotency_key,
                    })
                    .await?;
                Ok(SendActionResponse::LnurlPay(response))
            }
        }
    }

    /// Executes an LNURL-withdraw from a [`ReceiveAction`].
    ///
    /// This is a convenience wrapper around [`lnurl_withdraw()`](BreezSdk::lnurl_withdraw)
    /// that extracts the withdraw details from the action.
    pub async fn withdraw(
        &self,
        action: ReceiveAction,
        amount_sats: u64,
        completion_timeout_secs: Option<u32>,
    ) -> Result<LnurlWithdrawResponse, SdkError> {
        match action {
            ReceiveAction::LnurlWithdraw { withdraw_details } => {
                self.lnurl_withdraw(LnurlWithdrawRequest {
                    amount_sats,
                    withdraw_request: withdraw_details,
                    completion_timeout_secs,
                })
                .await
            }
        }
    }

    /// Performs LNURL-auth from an [`AuthAction`].
    ///
    /// This is a convenience wrapper around [`lnurl_auth()`](BreezSdk::lnurl_auth)
    /// that extracts the request data from the action.
    pub async fn authenticate(&self, action: AuthAction) -> Result<LnurlCallbackStatus, SdkError> {
        self.lnurl_auth(action.request_data).await
    }
}
