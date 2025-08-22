use std::rc::Rc;

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    event::{EventListener, WasmEventListener},
    logger::{Logger, WasmTracingLayer},
    models::*,
    persist::Storage,
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

    subscriber.init();

    Ok(())
}

#[wasm_bindgen(js_name = "defaultConfig")]
pub fn default_config(network: Network) -> Config {
    breez_sdk_spark::default_config(network.into()).into()
}

#[wasm_bindgen(js_name = "defaultStorage")]
pub fn default_storage(data_dir: &str) -> WasmResult<Storage> {
    let storage = crate::platform::default_storage(data_dir)?;
    Ok(storage)
}

#[wasm_bindgen(js_name = "parse")]
pub async fn parse(input: &str) -> WasmResult<InputType> {
    Ok(breez_sdk_spark::parse(input).await?.into())
}

#[wasm_bindgen]
impl BreezSdk {
    #[wasm_bindgen(js_name = "addEventListener")]
    pub fn add_event_listener(&self, listener: EventListener) -> String {
        self.sdk
            .add_event_listener(Box::new(WasmEventListener { listener }))
    }

    #[wasm_bindgen(js_name = "removeEventListener")]
    pub fn remove_event_listener(&self, id: &str) -> bool {
        self.sdk.remove_event_listener(id)
    }

    #[wasm_bindgen(js_name = "disconnect")]
    pub fn disconnect(&self) -> WasmResult<()> {
        Ok(self.sdk.disconnect()?)
    }

    #[wasm_bindgen(js_name = "getInfo")]
    pub fn get_info(&self, request: GetInfoRequest) -> WasmResult<GetInfoResponse> {
        Ok(self.sdk.get_info(request.into())?.into())
    }

    #[wasm_bindgen(js_name = "prepareReceivePayment")]
    pub fn prepare_receive_payment(
        &self,
        request: PrepareReceivePaymentRequest,
    ) -> WasmResult<PrepareReceivePaymentResponse> {
        Ok(self.sdk.prepare_receive_payment(request.into())?.into())
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

    #[wasm_bindgen(js_name = "sendPayment")]
    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> WasmResult<SendPaymentResponse> {
        Ok(self.sdk.send_payment(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "syncWallet")]
    pub fn sync_wallet(&self, request: SyncWalletRequest) -> WasmResult<SyncWalletResponse> {
        Ok(self.sdk.sync_wallet(request.into())?.into())
    }

    #[wasm_bindgen(js_name = "listPayments")]
    pub fn list_payments(&self, request: ListPaymentsRequest) -> WasmResult<ListPaymentsResponse> {
        Ok(self.sdk.list_payments(request.into())?.into())
    }

    #[wasm_bindgen(js_name = "getPayment")]
    pub fn get_payment(&self, request: GetPaymentRequest) -> WasmResult<GetPaymentResponse> {
        Ok(self.sdk.get_payment(request.into())?.into())
    }
}
