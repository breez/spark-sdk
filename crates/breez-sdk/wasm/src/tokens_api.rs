use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        FetchConversionLimitsRequest, FetchConversionLimitsResponse, GetTokensMetadataRequest,
        GetTokensMetadataResponse,
    },
};

/// Sub-object for token operations.
///
/// Access via `wallet.tokens`.
#[wasm_bindgen(js_name = "TokensApi")]
pub struct TokensApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "TokensApi")]
impl TokensApi {
    /// Get metadata for tokens.
    pub async fn metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> WasmResult<GetTokensMetadataResponse> {
        Ok(self.sdk.get_tokens_metadata(request.into()).await?.into())
    }

    /// Fetch swap limits for Bitcoin ↔ Token conversions.
    #[wasm_bindgen(js_name = "swapLimits")]
    pub async fn swap_limits(
        &self,
        request: FetchConversionLimitsRequest,
    ) -> WasmResult<FetchConversionLimitsResponse> {
        Ok(self
            .sdk
            .fetch_conversion_limits(request.into())
            .await?
            .into())
    }
}
