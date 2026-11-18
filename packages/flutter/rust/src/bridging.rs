use std::sync::Arc;

use breez_sdk_spark::{
    BridgingConfig, CreateBridgeRequest, CreateBridgeResponse, GetBridgeRoutesRequest,
    GetBridgeRoutesResponse, GetBridgeStatusRequest, GetBridgeStatusResponse, SdkError,
};
use flutter_rust_bridge::frb;

pub struct BridgingClient {
    pub(crate) inner: Arc<breez_sdk_spark::BridgingClient>,
}

/// Builds a `BridgingClient` from an API key, with no seed or storage.
pub async fn new_bridging_client(config: BridgingConfig) -> Result<BridgingClient, SdkError> {
    Ok(BridgingClient {
        inner: breez_sdk_spark::new_bridging_client(config).await?,
    })
}

#[frb(sync)]
pub fn default_bridging_config(api_key: String) -> BridgingConfig {
    breez_sdk_spark::default_bridging_config(api_key)
}

impl BridgingClient {
    pub async fn get_routes(
        &self,
        request: GetBridgeRoutesRequest,
    ) -> Result<GetBridgeRoutesResponse, SdkError> {
        self.inner.get_routes(request).await
    }

    pub async fn create_bridge(
        &self,
        request: CreateBridgeRequest,
    ) -> Result<CreateBridgeResponse, SdkError> {
        self.inner.create_bridge(request).await
    }

    pub async fn get_bridge_status(
        &self,
        request: GetBridgeStatusRequest,
    ) -> Result<GetBridgeStatusResponse, SdkError> {
        self.inner.get_bridge_status(request).await
    }

    pub async fn bridge_from_cash_app(
        &self,
        request: CreateBridgeRequest,
    ) -> Result<CreateBridgeResponse, SdkError> {
        self.inner.bridge_from_cash_app(request).await
    }

    pub async fn bridge_to_cash_app(
        &self,
        request: CreateBridgeRequest,
    ) -> Result<CreateBridgeResponse, SdkError> {
        self.inner.bridge_to_cash_app(request).await
    }
}
