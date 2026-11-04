use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        BridgingConfig, CreateBridgeRequest, CreateBridgeResponse, GetBridgeRoutesRequest,
        GetBridgeRoutesResponse, GetBridgeStatusRequest, GetBridgeStatusResponse,
    },
};

/// Creates bridges: orders that pay a recipient from another chain or over
/// Lightning, with no funds moving through the SDK. Only available on mainnet.
#[wasm_bindgen]
pub struct BridgingClient {
    pub(crate) inner: Arc<breez_sdk_spark::BridgingClient>,
}

/// Builds a `BridgingClient` from an API key, with no seed or storage.
#[wasm_bindgen(js_name = "newBridgingClient")]
pub async fn new_bridging_client(config: BridgingConfig) -> WasmResult<BridgingClient> {
    Ok(BridgingClient {
        inner: breez_sdk_spark::new_bridging_client(config.into()).await?,
    })
}

/// Settings for `newBridgingClient` with every optional field unset.
#[wasm_bindgen(js_name = "defaultBridgingConfig")]
pub fn default_bridging_config(api_key: String) -> BridgingConfig {
    breez_sdk_spark::default_bridging_config(api_key).into()
}

#[wasm_bindgen]
impl BridgingClient {
    #[wasm_bindgen(js_name = "getRoutes")]
    pub async fn get_routes(
        &self,
        request: GetBridgeRoutesRequest,
    ) -> WasmResult<GetBridgeRoutesResponse> {
        Ok(self.inner.get_routes(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "createBridge")]
    pub async fn create_bridge(
        &self,
        request: CreateBridgeRequest,
    ) -> WasmResult<CreateBridgeResponse> {
        Ok(self.inner.create_bridge(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "getBridgeStatus")]
    pub async fn get_bridge_status(
        &self,
        request: GetBridgeStatusRequest,
    ) -> WasmResult<GetBridgeStatusResponse> {
        Ok(self.inner.get_bridge_status(request.into()).await?.into())
    }

    #[wasm_bindgen(js_name = "bridgeFromCashApp")]
    pub async fn bridge_from_cash_app(
        &self,
        request: CreateBridgeRequest,
    ) -> WasmResult<CreateBridgeResponse> {
        Ok(self
            .inner
            .bridge_from_cash_app(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "bridgeToCashApp")]
    pub async fn bridge_to_cash_app(
        &self,
        request: CreateBridgeRequest,
    ) -> WasmResult<CreateBridgeResponse> {
        Ok(self.inner.bridge_to_cash_app(request.into()).await?.into())
    }
}
