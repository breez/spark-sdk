use std::sync::Arc;

use breez_sdk_common::input::InputType;
use breez_sdk_spark::{
    CheckLightningAddressRequest, CheckMessageRequest, CheckMessageResponse, ClaimDepositRequest,
    ClaimDepositResponse, Config, ConnectRequest, GetInfoRequest, GetInfoResponse,
    GetPaymentRequest, GetPaymentResponse, GetTokensMetadataRequest, GetTokensMetadataResponse,
    LightningAddressInfo, ListFiatCurrenciesResponse, ListFiatRatesResponse, ListPaymentsRequest,
    ListPaymentsResponse, ListUnclaimedDepositsRequest, ListUnclaimedDepositsResponse,
    LnurlPayRequest, LnurlPayResponse, LnurlWithdrawRequest, LnurlWithdrawResponse, LogEntry,
    PrepareLnurlPayResponse, PrepareSendPaymentRequest, PrepareSendPaymentResponse,
    ReceivePaymentRequest, ReceivePaymentResponse, RefundDepositRequest, RefundDepositResponse,
    RegisterLightningAddressRequest, SdkError, SdkEvent, SendPaymentRequest, SendPaymentResponse,
    SignMessageRequest, SignMessageResponse, Storage, SyncWalletRequest, SyncWalletResponse,
    WaitForPaymentRequest, WaitForPaymentResponse,
};
use flutter_rust_bridge::frb;

use crate::events::BindingEventListener;
use crate::frb_generated::StreamSink;
use crate::logger::BindingLogger;

pub async fn connect(request: ConnectRequest) -> Result<BreezSdk, SdkError> {
    let sdk = breez_sdk_spark::connect(request).await?;
    Ok(BreezSdk {
        inner: Arc::new(sdk),
    })
}

#[frb(sync)]
pub fn default_config(network: Network) -> Config {
    breez_sdk_spark::default_config(network)
}

#[frb(sync)]
pub fn default_storage(data_dir: String) -> Result<Arc<dyn Storage>, SdkError> {
    breez_sdk_spark::default_storage(data_dir)
}

#[frb(sync)]
pub fn init_logging(
    log_dir: Option<String>,
    app_logger: StreamSink<LogEntry>,
    log_filter: Option<String>,
) -> Result<(), SdkError> {
    let app_logger: Box<dyn Logger> = Box::new(BindingLogger { logger: app_logger });
    breez_sdk_spark::init_logging(log_dir, Some(app_logger), log_filter)
}

pub struct BreezSdk {
    pub(crate) inner: Arc<breez_sdk_spark::BreezSdk>,
}

impl BreezSdk {
    pub async fn add_event_listener(&self, listener: StreamSink<SdkEvent>) -> String {
        self.inner
            .add_event_listener(Box::new(BindingEventListener { listener }))
            .await
    }

    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.inner.remove_event_listener(id).await
    }

    pub async fn disconnect(&self) -> Result<(), SdkError> {
        self.inner.disconnect().await
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, SdkError> {
        self.inner.parse(input).await
    }

    pub async fn get_info(&self, request: GetInfoRequest) -> Result<GetInfoResponse, SdkError> {
        self.inner.get_info(request).await
    }

    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> Result<ReceivePaymentResponse, SdkError> {
        self.inner.receive_payment(request).await
    }

    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> Result<PrepareLnurlPayResponse, SdkError> {
        self.inner.prepare_lnurl_pay(request).await
    }

    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> Result<LnurlPayResponse, SdkError> {
        self.inner.lnurl_pay(request).await
    }

    pub async fn lnurl_withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> Result<LnurlWithdrawResponse, SdkError> {
        self.inner.lnurl_withdraw(request).await
    }

    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        self.inner.prepare_send_payment(request).await
    }

    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        self.inner.send_payment(request).await
    }

    pub async fn sync_wallet(
        &self,
        request: SyncWalletRequest,
    ) -> Result<SyncWalletResponse, SdkError> {
        self.inner.sync_wallet(request).await
    }

    pub async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, SdkError> {
        self.inner.list_payments(request).await
    }

    pub async fn get_payment(
        &self,
        request: GetPaymentRequest,
    ) -> Result<GetPaymentResponse, SdkError> {
        self.inner.get_payment(request).await
    }

    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> Result<ClaimDepositResponse, SdkError> {
        self.inner.claim_deposit(request).await
    }

    pub async fn refund_deposit(
        &self,
        request: RefundDepositRequest,
    ) -> Result<RefundDepositResponse, SdkError> {
        self.inner.refund_deposit(request).await
    }

    pub async fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        self.inner.list_unclaimed_deposits(request).await
    }

    pub async fn check_lightning_address_available(
        &self,
        request: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        self.inner.check_lightning_address_available(request).await
    }

    pub async fn get_lightning_address(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        self.inner.get_lightning_address().await
    }

    pub async fn register_lightning_address(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        self.inner.register_lightning_address(request).await
    }

    pub async fn delete_lightning_address(&self) -> Result<(), SdkError> {
        self.inner.delete_lightning_address().await
    }

    pub async fn list_fiat_currencies(&self) -> Result<ListFiatCurrenciesResponse, SdkError> {
        self.inner.list_fiat_currencies().await
    }

    pub async fn list_fiat_rates(&self) -> Result<ListFiatRatesResponse, SdkError> {
        self.inner.list_fiat_rates().await
    }

    pub async fn wait_for_payment(
        &self,
        request: WaitForPaymentRequest,
    ) -> Result<WaitForPaymentResponse, SdkError> {
        self.inner.wait_for_payment(request).await
    }

    pub async fn get_tokens_metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> Result<GetTokensMetadataResponse, SdkError> {
        self.inner.get_tokens_metadata(request).await
    }

    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> Result<SignMessageResponse, SdkError> {
        self.inner.sign_message(request).await
    }

    pub async fn check_message(
        &self,
        request: CheckMessageRequest,
    ) -> Result<CheckMessageResponse, SdkError> {
        self.inner.check_message(request).await
    }
}
