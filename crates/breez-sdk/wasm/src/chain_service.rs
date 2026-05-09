use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::models::{
    Credentials, Network,
    chain_service::{ChainApiType, RecommendedFees, TxStatus, Utxo},
};

/// Rust-built implementation of the JS `BitcoinChainService` interface.
///
/// Returned by factories like [`new_rest_chain_service`]; users see it as a
/// `BitcoinChainService` and pass it to `withChainService`. Pass the same
/// instance to multiple `SdkBuilder`s to share a single underlying HTTP
/// client (and its connection pool) across SDK instances.
#[wasm_bindgen]
pub struct BitcoinChainServiceHandle {
    pub(crate) inner: Arc<dyn breez_sdk_spark::BitcoinChainService>,
}

#[wasm_bindgen]
impl BitcoinChainServiceHandle {
    #[wasm_bindgen(js_name = "getAddressUtxos")]
    pub async fn get_address_utxos(&self, address: String) -> Result<JsValue, JsValue> {
        let utxos = self
            .inner
            .get_address_utxos(address)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let utxos: Vec<Utxo> = utxos.into_iter().map(Into::into).collect();
        serde_wasm_bindgen::to_value(&utxos).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "getTransactionStatus")]
    pub async fn get_transaction_status(&self, txid: String) -> Result<JsValue, JsValue> {
        let status = self
            .inner
            .get_transaction_status(txid)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let status: TxStatus = status.into();
        serde_wasm_bindgen::to_value(&status).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "getTransactionHex")]
    pub async fn get_transaction_hex(&self, txid: String) -> Result<JsValue, JsValue> {
        let hex = self
            .inner
            .get_transaction_hex(txid)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(JsValue::from_str(&hex))
    }

    #[wasm_bindgen(js_name = "broadcastTransaction")]
    pub async fn broadcast_transaction(&self, tx: String) -> Result<(), JsValue> {
        self.inner
            .broadcast_transaction(tx)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = "recommendedFees")]
    pub async fn recommended_fees(&self) -> Result<JsValue, JsValue> {
        let fees = self
            .inner
            .recommended_fees()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let fees: RecommendedFees = fees.into();
        serde_wasm_bindgen::to_value(&fees).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

/// Constructs a shareable REST-based Bitcoin chain service.
///
/// Pass the returned chain service to multiple `SdkBuilder`s via
/// `withChainService` to reuse one HTTP client across SDK instances. All
/// SDKs sharing the chain service must use the same `network`.
///
/// For one-off, non-shared use, prefer `withRestChainService`.
#[wasm_bindgen(
    js_name = "newRestChainService",
    unchecked_return_type = "BitcoinChainService"
)]
#[must_use]
pub async fn new_rest_chain_service(
    url: String,
    network: Network,
    api_type: ChainApiType,
    credentials: Option<Credentials>,
) -> BitcoinChainServiceHandle {
    BitcoinChainServiceHandle {
        inner: breez_sdk_spark::new_rest_chain_service(
            url,
            network.into(),
            api_type.into(),
            credentials.map(Into::into),
        )
        .await,
    }
}
