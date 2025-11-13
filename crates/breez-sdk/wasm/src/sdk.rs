use std::rc::Rc;

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    event::{EventListener, WasmEventListener},
    logger::{Logger, WasmTracingLayer},
    models::*,
    sdk_builder::SdkBuilder,
};

#[wasm_bindgen]
pub struct BreezSdk {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
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

#[wasm_bindgen(js_name = "connect")]
pub async fn connect(request: ConnectRequest) -> WasmResult<BreezSdk> {
    let builder = SdkBuilder::new(request.config, request.seed)
        .with_default_storage(request.storage_dir)
        .await?;
    let sdk = builder.build().await?;
    Ok(sdk)
}

#[wasm_bindgen(js_name = "defaultConfig")]
pub fn default_config(network: Network) -> Config {
    breez_sdk_spark::default_config(network.into()).into()
}

#[wasm_bindgen]
impl BreezSdk {
    #[wasm_bindgen(js_name = "addEventListener")]
    pub async fn add_event_listener(&self, listener: EventListener) -> String {
        self.sdk
            .add_event_listener(Box::new(WasmEventListener { listener }))
            .await
    }

    #[wasm_bindgen(js_name = "removeEventListener")]
    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.sdk.remove_event_listener(id).await
    }

    #[wasm_bindgen(js_name = "disconnect")]
    pub async fn disconnect(&self) -> WasmResult<()> {
        Ok(self.sdk.disconnect().await?)
    }

    #[wasm_bindgen(js_name = "parse")]
    pub async fn parse(&self, input: &str) -> WasmResult<InputType> {
        Ok(self.sdk.parse(input).await?.into())
    }

    #[wasm_bindgen(js_name = "getInfo")]
    pub async fn get_info(&self, request: GetInfoRequest) -> WasmResult<GetInfoResponse> {
        Ok(self.sdk.get_info(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "receivePayment")]
    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> WasmResult<ReceivePaymentResponse> {
        Ok(self.sdk.receive_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "prepareSendPayment")]
    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> WasmResult<PrepareSendPaymentResponse> {
        Ok(self.sdk.prepare_send_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "prepareLnurlPay")]
    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> WasmResult<PrepareLnurlPayResponse> {
        Ok(self.sdk.prepare_lnurl_pay(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "lnurlPay")]
    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> WasmResult<LnurlPayResponse> {
        Ok(self.sdk.lnurl_pay(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "lnurlWithdraw")]
    pub async fn lnurl_withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> WasmResult<LnurlWithdrawResponse> {
        Ok(self.sdk.lnurl_withdraw(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "sendPayment")]
    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> WasmResult<SendPaymentResponse> {
        Ok(self.sdk.send_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "syncWallet")]
    pub async fn sync_wallet(&self, request: SyncWalletRequest) -> WasmResult<SyncWalletResponse> {
        Ok(self.sdk.sync_wallet(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "listPayments")]
    pub async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> WasmResult<ListPaymentsResponse> {
        Ok(self.sdk.list_payments(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "getPayment")]
    pub async fn get_payment(&self, request: GetPaymentRequest) -> WasmResult<GetPaymentResponse> {
        Ok(self.sdk.get_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "claimDeposit")]
    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> WasmResult<ClaimDepositResponse> {
        Ok(self.sdk.claim_deposit(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "refundDeposit")]
    pub async fn refund_deposit(
        &self,
        request: RefundDepositRequest,
    ) -> WasmResult<RefundDepositResponse> {
        Ok(self.sdk.refund_deposit(request.into()).await?.into())
    }

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

    #[wasm_bindgen(js_name = "getLightningAddress")]
    pub async fn get_lightning_address(&self) -> WasmResult<Option<LightningAddressInfo>> {
        Ok(self
            .sdk
            .get_lightning_address()
            .await?
            .map(|resp| resp.into()))
    }

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

    #[wasm_bindgen(js_name = "deleteLightningAddress")]
    pub async fn delete_lightning_address(&self) -> WasmResult<()> {
        Ok(self.sdk.delete_lightning_address().await?)
    }

    #[wasm_bindgen(js_name = "listFiatCurrencies")]
    pub async fn list_fiat_currencies(&self) -> WasmResult<ListFiatCurrenciesResponse> {
        Ok(self.sdk.list_fiat_currencies().await?.into())
    }

    #[wasm_bindgen(js_name = "listFiatRates")]
    pub async fn list_fiat_rates(&self) -> WasmResult<ListFiatRatesResponse> {
        Ok(self.sdk.list_fiat_rates().await?.into())
    }

    #[wasm_bindgen(js_name = "waitForPayment")]
    pub async fn wait_for_payment(
        &self,
        request: WaitForPaymentRequest,
    ) -> WasmResult<WaitForPaymentResponse> {
        Ok(self.sdk.wait_for_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "getTokensMetadata")]
    pub async fn get_tokens_metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> WasmResult<GetTokensMetadataResponse> {
        Ok(self.sdk.get_tokens_metadata(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "signMessage")]
    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> WasmResult<SignMessageResponse> {
        Ok(self.sdk.sign_message(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "checkMessage")]
    pub async fn check_message(
        &self,
        request: CheckMessageRequest,
    ) -> WasmResult<CheckMessageResponse> {
        Ok(self.sdk.check_message(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "getUserSettings")]
    pub async fn get_user_settings(&self) -> WasmResult<UserSettings> {
        Ok(self.sdk.get_user_settings().await?.into())
    }

    #[wasm_bindgen(js_name = "updateUserSettings")]
    pub async fn update_user_settings(&self, request: UpdateUserSettingsRequest) -> WasmResult<()> {
        Ok(self.sdk.update_user_settings(request.into()).await?)
    }
}
