use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use crate::models::error::js_error_to_chain_service_error;

#[macros::extern_wasm_bindgen(breez_sdk_spark::TxStatus)]
pub struct TxStatus {
    pub confirmed: bool,
    pub block_height: Option<u32>,
    pub block_time: Option<u64>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::Utxo)]
pub struct Utxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub status: TxStatus,
}

pub struct WasmBitcoinChainService {
    pub inner: BitcoinChainService,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmBitcoinChainService {}
unsafe impl Sync for WasmBitcoinChainService {}

#[macros::async_trait]
impl breez_sdk_spark::BitcoinChainService for WasmBitcoinChainService {
    async fn get_address_utxos(
        &self,
        address: String,
    ) -> Result<Vec<breez_sdk_spark::Utxo>, breez_sdk_spark::ChainServiceError> {
        let promise = self
            .inner
            .get_address_utxos(address)
            .map_err(js_error_to_chain_service_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_chain_service_error)?;
        let utxos: Vec<Utxo> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| breez_sdk_spark::ChainServiceError::Generic(e.to_string()))?;
        Ok(utxos.into_iter().map(|p| p.into()).collect())
    }

    async fn get_transaction_status(
        &self,
        txid: String,
    ) -> Result<breez_sdk_spark::TxStatus, breez_sdk_spark::ChainServiceError> {
        let promise = self
            .inner
            .get_transaction_status(txid)
            .map_err(js_error_to_chain_service_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_chain_service_error)?;
        let tx_status: TxStatus = serde_wasm_bindgen::from_value(result)
            .map_err(|e| breez_sdk_spark::ChainServiceError::Generic(e.to_string()))?;
        Ok(tx_status.into())
    }

    async fn get_transaction_hex(
        &self,
        txid: String,
    ) -> Result<String, breez_sdk_spark::ChainServiceError> {
        let promise = self
            .inner
            .get_transaction_hex(txid)
            .map_err(js_error_to_chain_service_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_chain_service_error)?;
        let tx_hex: String = serde_wasm_bindgen::from_value(result)
            .map_err(|e| breez_sdk_spark::ChainServiceError::Generic(e.to_string()))?;
        Ok(tx_hex)
    }

    async fn broadcast_transaction(
        &self,
        tx: String,
    ) -> Result<(), breez_sdk_spark::ChainServiceError> {
        let promise = self
            .inner
            .broadcast_transaction(tx)
            .map_err(js_error_to_chain_service_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_chain_service_error)?;
        Ok(())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface BitcoinChainService {
    getAddressUtxos(address: string): Promise<Utxo[]>;
    getTransactionStatus(txid: string): Promise<TxStatus>;
    getTransactionHex(txid: string): Promise<string>;
    broadcastTransaction(tx: string): Promise<void>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "BitcoinChainService")]
    pub type BitcoinChainService;

    #[wasm_bindgen(structural, method, js_name = "getAddressUtxos", catch)]
    pub fn get_address_utxos(
        this: &BitcoinChainService,
        address: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getTransactionStatus", catch)]
    pub fn get_transaction_status(
        this: &BitcoinChainService,
        txid: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "getTransactionHex", catch)]
    pub fn get_transaction_hex(
        this: &BitcoinChainService,
        txid: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "broadcastTransaction", catch)]
    pub fn broadcast_transaction(
        this: &BitcoinChainService,
        tx: String,
    ) -> Result<Promise, JsValue>;
}
