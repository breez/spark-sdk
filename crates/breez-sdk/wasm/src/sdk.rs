use std::rc::Rc;

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use wasm_bindgen::prelude::*;

use crate::{
    deposits_api::DepositsApi,
    error::WasmResult,
    event::{EventListener, WasmEventListener, WasmFilteredEventListener},
    events_api::EventsApi,
    fiat_api::FiatApi,
    issuer::TokenIssuer,
    lightning_address_api::LightningAddressApi,
    lnurl_api::LnurlApi,
    logger::{Logger, WasmTracingLayer},
    message_api::MessageApi,
    models::{chain_service::RecommendedFees, *},
    optimization_api::OptimizationApi,
    payment_intent::PaymentIntent,
    payments_api::PaymentsApi,
    sdk_builder::SdkBuilder,
    settings_api::SettingsApi,
    tokens_api::TokensApi,
};

#[wasm_bindgen(js_name = "BreezClient")]
pub struct BreezClient {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezClient>,
}

/// Backward-compat aliases so existing imports still work.
#[wasm_bindgen(typescript_custom_section)]
const BREEZ_SDK_ALIAS: &'static str = r#"
/** @deprecated Use `BreezClient` instead. */
export type BreezSdk = BreezClient;
/** @deprecated Use `BreezClient` instead. */
export type Wallet = BreezClient;"#;

/// Destination for `preparePayment()`: either a raw string or a pre-parsed `InputType`.
#[wasm_bindgen(typescript_custom_section)]
const PAYMENT_DESTINATION_TYPE: &'static str = r#"
/**
 * A payment destination: either a raw string (invoice, address) or a pre-parsed
 * `InputType` from a prior `parseInput()` call.
 *
 * **LNURL-Pay and Lightning Address** destinations **must** be pre-parsed via
 * `parseInput()` first — passing a raw LNURL/Lightning address string will throw.
 * This enforces the LUD-06 wallet flow: parse first to discover min/max sendable
 * and description metadata, show to user, then pass the parsed `InputType`.
 *
 * For non-LNURL destinations (Bolt11 invoices, Bitcoin addresses, Spark addresses),
 * either a raw string or parsed `InputType` is accepted.
 */
export type PaymentDestination = string | InputType;"#;

// ── Sub-object getters (sync — just Rc::clone) ─────────────────────
#[wasm_bindgen(js_class = "BreezClient")]
impl BreezClient {
    /// Payment query API.
    #[wasm_bindgen(getter)]
    pub fn payments(&self) -> PaymentsApi {
        PaymentsApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Lightning address management API.
    #[wasm_bindgen(getter, js_name = "lightningAddress")]
    pub fn lightning_address(&self) -> LightningAddressApi {
        LightningAddressApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Deposit management API.
    #[wasm_bindgen(getter)]
    pub fn deposits(&self) -> DepositsApi {
        DepositsApi {
            sdk: self.sdk.clone(),
        }
    }

    /// User settings API.
    #[wasm_bindgen(getter, js_name = "userSettings")]
    pub fn user_settings(&self) -> SettingsApi {
        SettingsApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Message signing API.
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> MessageApi {
        MessageApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Token operations API.
    #[wasm_bindgen(getter)]
    pub fn tokens(&self) -> TokensApi {
        TokensApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Token issuer API (for token issuers only).
    #[wasm_bindgen(getter, js_name = "tokenIssuer")]
    pub fn token_issuer(&self) -> TokenIssuer {
        let token_issuer = self.sdk.get_token_issuer();
        TokenIssuer {
            token_issuer: Rc::new(token_issuer),
        }
    }

    /// Leaf optimization API.
    #[wasm_bindgen(getter)]
    pub fn optimization(&self) -> OptimizationApi {
        OptimizationApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Event listener management API.
    #[wasm_bindgen(getter)]
    pub fn events(&self) -> EventsApi {
        EventsApi {
            sdk: self.sdk.clone(),
        }
    }

    /// LNURL operations API (auth, withdraw).
    #[wasm_bindgen(getter)]
    pub fn lnurl(&self) -> LnurlApi {
        LnurlApi {
            sdk: self.sdk.clone(),
        }
    }

    /// Fiat data API (exchange rates, currencies).
    #[wasm_bindgen(getter)]
    pub fn fiat(&self) -> FiatApi {
        FiatApi {
            sdk: self.sdk.clone(),
        }
    }

    /// The wallet's identity public key as a hex string.
    ///
    /// This is synchronous — no network call needed.
    #[wasm_bindgen(getter)]
    pub fn pubkey(&self) -> String {
        self.sdk.identity_pubkey()
    }
}

#[wasm_bindgen(js_name = "initLogging")]
pub async fn init_logging(logger: Logger, filter: Option<String>) -> WasmResult<()> {
    crate::logger::WASM_LOGGER.set(Some(logger));

    let filter = EnvFilter::new(filter.unwrap_or(
        "debug,h2=warn,rustls=warn,rustyline=warn,hyper=warn,hyper_util=warn,tower=warn,Connection=warn,tonic=warn".to_string(),
    ));
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(WasmTracingLayer {});

    subscriber.try_init()?;

    Ok(())
}

/// @deprecated Use `Breez.connect()` instead.
#[wasm_bindgen(js_name = "connect")]
pub async fn connect(request: ConnectRequest) -> WasmResult<BreezClient> {
    let builder = SdkBuilder::new(request.config, request.seed)
        .with_default_storage(request.storage_dir)
        .await?;
    let sdk = builder.build().await?;
    Ok(sdk)
}

/// @deprecated Use `Breez.connect()` instead.
#[wasm_bindgen(js_name = "connectWithSigner")]
pub async fn connect_with_signer(
    config: Config,
    signer: crate::signer::JsExternalSigner,
    storage_dir: String,
) -> WasmResult<BreezClient> {
    let builder = SdkBuilder::new_with_signer(config, signer)
        .with_default_storage(storage_dir)
        .await?;
    let sdk = builder.build().await?;
    Ok(sdk)
}

/// @deprecated Use `ClientConfig` with defaults instead.
#[allow(deprecated)]
#[wasm_bindgen(js_name = "defaultConfig")]
pub fn default_config(network: Network) -> Config {
    breez_sdk_spark::default_config(network.into()).into()
}

/// Creates a default external signer from a mnemonic phrase.
///
/// This creates a signer that can be used with `connectWithSigner` or `SdkBuilder.newWithSigner`.
#[wasm_bindgen(js_name = "getSparkStatus")]
pub async fn get_spark_status() -> WasmResult<SparkStatus> {
    Ok(breez_sdk_spark::get_spark_status().await?.into())
}

#[wasm_bindgen(js_name = "defaultExternalSigner")]
pub fn default_external_signer(
    mnemonic: String,
    passphrase: Option<String>,
    network: Network,
    key_set_config: Option<crate::models::KeySetConfig>,
) -> WasmResult<crate::signer::DefaultSigner> {
    let signer = breez_sdk_spark::default_external_signer(
        mnemonic,
        passphrase,
        network.into(),
        key_set_config.map(|k| k.into()),
    )?;

    Ok(crate::signer::DefaultSigner::new(signer))
}

#[wasm_bindgen(js_class = "BreezClient")]
#[allow(deprecated)] // WASM methods delegate to deprecated core methods during migration
impl BreezClient {
    /// @deprecated Use `client.events.add()` instead.
    #[wasm_bindgen(js_name = "addEventListener")]
    pub async fn add_event_listener(&self, listener: EventListener) -> String {
        self.sdk
            .add_event_listener(Box::new(WasmEventListener { listener }))
            .await
    }

    /// @deprecated Use `client.events.remove()` instead.
    #[wasm_bindgen(js_name = "removeEventListener")]
    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.sdk.remove_event_listener(id).await
    }

    /// @deprecated Use `client.events.on()` instead.
    #[wasm_bindgen(js_name = "on")]
    pub async fn on(&self, event_type: &str, callback: js_sys::Function) -> WasmResult<String> {
        // Build a filter based on the event type string
        let filter: fn(&breez_sdk_spark::SdkEvent) -> bool = match event_type {
            "payment" => breez_sdk_spark::SdkEvent::is_payment,
            "paymentSucceeded" => |e| matches!(e, breez_sdk_spark::SdkEvent::PaymentSucceeded { .. }),
            "paymentPending" => |e| matches!(e, breez_sdk_spark::SdkEvent::PaymentPending { .. }),
            "paymentFailed" => |e| matches!(e, breez_sdk_spark::SdkEvent::PaymentFailed { .. }),
            "synced" => breez_sdk_spark::SdkEvent::is_synced,
            _ => {
                return Err(breez_sdk_spark::SdkError::InvalidInput(format!(
                    "Unknown event type: \"{event_type}\". Supported: payment, paymentSucceeded, paymentPending, paymentFailed, synced"
                )).into());
            }
        };

        let listener = WasmFilteredEventListener { filter, callback };
        let id = self.sdk.add_event_listener(Box::new(listener)).await;
        Ok(id)
    }

    #[wasm_bindgen(js_name = "disconnect")]
    pub async fn disconnect(&self) -> WasmResult<()> {
        Ok(self.sdk.disconnect().await?)
    }

    /// @deprecated Use standalone `parseInput()` function instead.
    #[wasm_bindgen(js_name = "parse")]
    pub async fn parse(&self, input: &str) -> WasmResult<InputType> {
        Ok(self.sdk.parse(input).await?.into())
    }

    /// Prepare a payment to the given destination.
    ///
    /// The `destination` can be:
    /// - A **string** (invoice, address) — parsed internally.
    /// - A pre-parsed **`InputType`** object from a prior `parseInput()` / `parse()` call.
    ///
    /// **LNURL-Pay / Lightning Address** destinations **must** be pre-parsed via
    /// `parseInput()` first — passing a raw LNURL/Lightning address string will
    /// throw an error. This enforces the [LUD-06](https://github.com/lnurl/luds/blob/luds/06.md)
    /// wallet flow: parse → show metadata (min/max sendable, description) → user
    /// selects amount → preparePayment(parsedInput, { amountSats }).
    ///
    /// Returns a `PaymentIntent` that can be inspected (`paymentType`, `amount`,
    /// `fee`, `feeSats`) and then sent with `send()`.
    ///
    /// Nothing is committed or reserved — this is a fee estimate + validation step.
    #[wasm_bindgen(js_name = "preparePayment")]
    pub async fn prepare_payment(
        &self,
        destination: JsValue,
        options: Option<PrepareOptions>,
    ) -> WasmResult<PaymentIntent> {
        let core_destination = if destination.is_string() {
            let s = destination.as_string().ok_or_else(|| {
                breez_sdk_spark::SdkError::InvalidInput(
                    "destination must be a string or InputType".to_string(),
                )
            })?;
            breez_sdk_spark::PaymentDestination::Raw { destination: s }
        } else {
            // Try to deserialize as InputType (passed from a prior parseInput() call)
            let input: InputType = serde_wasm_bindgen::from_value(destination).map_err(|e| {
                breez_sdk_spark::SdkError::InvalidInput(format!(
                    "destination must be a string or InputType object: {e}"
                ))
            })?;
            let core_input: breez_sdk_spark::InputType = input.into();
            breez_sdk_spark::PaymentDestination::Parsed { input: core_input }
        };
        let options = options.map(Into::into);
        let prepared = self
            .sdk
            .prepare_from_destination(core_destination, options)
            .await?;
        // Decompose Arc-based PreparedPayment and re-wrap with Rc for WASM
        let (_arc_sdk, data) = prepared.into_parts();
        let inner = breez_sdk_spark::PreparedPayment::new(self.sdk.clone(), data);
        Ok(PaymentIntent { inner })
    }


    /// Generate a payment request (invoice, address) to receive funds.
    #[wasm_bindgen(js_name = "receive")]
    pub async fn receive(&self, options: ReceiveOptions) -> WasmResult<ReceiveResult> {
        Ok(self.sdk.receive(options.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "getInfo")]
    pub async fn get_info(&self, request: Option<GetInfoRequest>) -> WasmResult<GetInfoResponse> {
        let core_request = match request {
            Some(r) => r.into(),
            None => breez_sdk_spark::GetInfoRequest {
                ensure_synced: None,
            },
        };
        Ok(self.sdk.get_info(core_request).await?.into())
    }

    /// Convenience method to get the wallet's balance in sats.
    #[wasm_bindgen(js_name = "getBalance")]
    pub async fn get_balance(&self) -> WasmResult<u64> {
        let info = self
            .sdk
            .get_info(breez_sdk_spark::GetInfoRequest {
                ensure_synced: None,
            })
            .await?;
        Ok(info.balance_sats)
    }

    /// @deprecated Use `receive()` instead.
    #[wasm_bindgen(js_name = "receivePayment")]
    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> WasmResult<ReceivePaymentResponse> {
        Ok(self.sdk.receive_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "claimHtlcPayment")]
    pub async fn claim_htlc_payment(
        &self,
        request: ClaimHtlcPaymentRequest,
    ) -> WasmResult<ClaimHtlcPaymentResponse> {
        Ok(self.sdk.claim_htlc_payment(request.into()).await?.into())
    }

    /// @deprecated Use `preparePayment()` instead.
    #[wasm_bindgen(js_name = "prepareSendPayment")]
    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> WasmResult<PrepareSendPaymentResponse> {
        Ok(self.sdk.prepare_send_payment(request.into()).await?.into())
    }

    /// @deprecated Use `preparePayment()` instead.
    #[wasm_bindgen(js_name = "prepareLnurlPay")]
    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> WasmResult<PrepareLnurlPayResponse> {
        Ok(self.sdk.prepare_lnurl_pay(request.into()).await?.into())
    }

    /// @deprecated Use `preparePayment()` + `PaymentIntent.send()` instead.
    #[wasm_bindgen(js_name = "lnurlPay")]
    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> WasmResult<LnurlPayResponse> {
        Ok(self.sdk.lnurl_pay(request.into()).await?.into())
    }

    /// @deprecated Use `client.lnurl.withdraw()` instead.
    #[wasm_bindgen(js_name = "lnurlWithdraw")]
    pub async fn lnurl_withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> WasmResult<LnurlWithdrawResponse> {
        Ok(self.sdk.lnurl_withdraw(request.into()).await?.into())
    }

    /// @deprecated Use `client.lnurl.auth()` instead.
    #[wasm_bindgen(js_name = "lnurlAuth")]
    pub async fn lnurl_auth(
        &self,
        request_data: LnurlAuthRequestDetails,
    ) -> WasmResult<LnurlCallbackStatus> {
        Ok(self.sdk.lnurl_auth(request_data.into()).await?.into())
    }

    /// @deprecated Use `sendPayment()` or `preparePayment()` + `send()` instead.
    #[wasm_bindgen(js_name = "sendPaymentLegacy")]
    pub async fn send_payment_legacy(
        &self,
        request: SendPaymentRequest,
    ) -> WasmResult<SendPaymentResponse> {
        Ok(self.sdk.send_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "syncWallet")]
    pub async fn sync_wallet(&self, request: SyncWalletRequest) -> WasmResult<SyncWalletResponse> {
        Ok(self.sdk.sync_wallet(request.into()).await?.into())
    }

    /// @deprecated Use `client.payments.list()` instead.
    #[wasm_bindgen(js_name = "listPayments")]
    pub async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> WasmResult<ListPaymentsResponse> {
        Ok(self.sdk.list_payments(request.into()).await?.into())
    }

    /// @deprecated Use `client.payments.get()` instead.
    #[wasm_bindgen(js_name = "getPayment")]
    pub async fn get_payment(&self, request: GetPaymentRequest) -> WasmResult<GetPaymentResponse> {
        Ok(self.sdk.get_payment(request.into()).await?.into())
    }

    /// @deprecated Use `client.deposits.claim()` instead.
    #[wasm_bindgen(js_name = "claimDeposit")]
    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> WasmResult<ClaimDepositResponse> {
        Ok(self.sdk.claim_deposit(request.into()).await?.into())
    }

    /// @deprecated Use `client.deposits.refund()` instead.
    #[wasm_bindgen(js_name = "refundDeposit")]
    pub async fn refund_deposit(
        &self,
        request: RefundDepositRequest,
    ) -> WasmResult<RefundDepositResponse> {
        Ok(self.sdk.refund_deposit(request.into()).await?.into())
    }

    /// @deprecated Use `client.deposits.listUnclaimed()` instead.
    #[wasm_bindgen(js_name = "listUnclaimedDeposits")]
    pub async fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> WasmResult<ListUnclaimedDepositsResponse> {
        Ok(self
            .sdk
            .list_unclaimed_deposits(request.into())
            .await?
            .into())
    }

    /// @deprecated Use `client.lightningAddress.checkAvailable()` instead.
    #[wasm_bindgen(js_name = "checkLightningAddressAvailable")]
    pub async fn check_lightning_address_available(
        &self,
        request: CheckLightningAddressRequest,
    ) -> WasmResult<bool> {
        Ok(self
            .sdk
            .check_lightning_address_available(request.into())
            .await?)
    }

    /// @deprecated Use `client.lightningAddress.get()` instead.
    #[wasm_bindgen(js_name = "getLightningAddress")]
    pub async fn get_lightning_address(&self) -> WasmResult<Option<LightningAddressInfo>> {
        Ok(self
            .sdk
            .get_lightning_address()
            .await?
            .map(|resp| resp.into()))
    }

    /// @deprecated Use `client.lightningAddress.register()` instead.
    #[wasm_bindgen(js_name = "registerLightningAddress")]
    pub async fn register_lightning_address(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> WasmResult<LightningAddressInfo> {
        Ok(self
            .sdk
            .register_lightning_address(request.into())
            .await?
            .into())
    }

    /// @deprecated Use `client.lightningAddress.delete()` instead.
    #[wasm_bindgen(js_name = "deleteLightningAddress")]
    pub async fn delete_lightning_address(&self) -> WasmResult<()> {
        Ok(self.sdk.delete_lightning_address().await?)
    }

    /// @deprecated Use standalone `Fiat.currencies()` or keep for convenience.
    #[wasm_bindgen(js_name = "listFiatCurrencies")]
    pub async fn list_fiat_currencies(&self) -> WasmResult<ListFiatCurrenciesResponse> {
        Ok(self.sdk.list_fiat_currencies().await?.into())
    }

    /// @deprecated Use standalone `Fiat.rates()` or keep for convenience.
    #[wasm_bindgen(js_name = "listFiatRates")]
    pub async fn list_fiat_rates(&self) -> WasmResult<ListFiatRatesResponse> {
        Ok(self.sdk.list_fiat_rates().await?.into())
    }

    /// Get the recommended BTC fees.
    ///
    /// This is the canonical location for recommended fees on the client.
    #[wasm_bindgen(js_name = "recommendedFees")]
    pub async fn recommended_fees(&self) -> WasmResult<RecommendedFees> {
        Ok(self.sdk.recommended_fees().await?.into())
    }

    /// @deprecated Use `client.tokens.metadata()` instead.
    #[wasm_bindgen(js_name = "getTokensMetadata")]
    pub async fn get_tokens_metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> WasmResult<GetTokensMetadataResponse> {
        Ok(self.sdk.get_tokens_metadata(request.into()).await?.into())
    }

    /// @deprecated Use `client.message.sign()` instead.
    #[wasm_bindgen(js_name = "signMessage")]
    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> WasmResult<SignMessageResponse> {
        Ok(self.sdk.sign_message(request.into()).await?.into())
    }

    /// @deprecated Use standalone `verifyMessage()` instead.
    #[wasm_bindgen(js_name = "checkMessage")]
    pub async fn check_message(
        &self,
        request: CheckMessageRequest,
    ) -> WasmResult<CheckMessageResponse> {
        Ok(self.sdk.check_message(request.into()).await?.into())
    }

    /// @deprecated Use `client.userSettings.get()` instead.
    #[wasm_bindgen(js_name = "getUserSettings")]
    pub async fn get_user_settings(&self) -> WasmResult<UserSettings> {
        Ok(self.sdk.get_user_settings().await?.into())
    }

    /// @deprecated Use `client.userSettings.update()` instead.
    #[wasm_bindgen(js_name = "updateUserSettings")]
    pub async fn update_user_settings(&self, request: UpdateUserSettingsRequest) -> WasmResult<()> {
        Ok(self.sdk.update_user_settings(request.into()).await?)
    }

    /// @deprecated Use `client.tokenIssuer` getter instead.
    #[wasm_bindgen(js_name = "getTokenIssuer")]
    pub fn get_token_issuer(&self) -> TokenIssuer {
        self.token_issuer()
    }

    /// @deprecated Use `client.optimization.start()` instead.
    #[wasm_bindgen(js_name = "startLeafOptimization")]
    pub fn start_leaf_optimization(&self) {
        self.sdk.start_leaf_optimization();
    }

    /// @deprecated Use `client.optimization.cancel()` instead.
    #[wasm_bindgen(js_name = "cancelLeafOptimization")]
    pub async fn cancel_leaf_optimization(&self) -> WasmResult<()> {
        Ok(self.sdk.cancel_leaf_optimization().await?)
    }

    /// @deprecated Use `client.optimization.progress` getter instead.
    #[wasm_bindgen(js_name = "getLeafOptimizationProgress")]
    pub fn get_leaf_optimization_progress(&self) -> OptimizationProgress {
        self.sdk.get_leaf_optimization_progress().into()
    }

    /// @deprecated Use `client.tokens.swapLimits()` instead.
    #[wasm_bindgen(js_name = "fetchConversionLimits")]
    pub async fn fetch_conversion_limits(
        &self,
        request: FetchConversionLimitsRequest,
    ) -> WasmResult<FetchConversionLimitsResponse> {
        Ok(self
            .sdk
            .fetch_conversion_limits(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "buyBitcoin")]
    pub async fn buy_bitcoin(&self, request: BuyBitcoinRequest) -> WasmResult<BuyBitcoinResponse> {
        Ok(self.sdk.buy_bitcoin(request.into()).await?.into())
    }
}
