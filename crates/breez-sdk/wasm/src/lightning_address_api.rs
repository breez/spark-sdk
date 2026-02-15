use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{LightningAddressInfo, RegisterLightningAddressRequest},
};

/// Sub-object for lightning address operations.
///
/// Access via `wallet.lightningAddress`.
#[wasm_bindgen(js_name = "LightningAddressApi")]
pub struct LightningAddressApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "LightningAddressApi")]
impl LightningAddressApi {
    /// Get the currently registered lightning address, if any.
    pub async fn get(&self) -> WasmResult<Option<LightningAddressInfo>> {
        Ok(self
            .sdk
            .get_lightning_address()
            .await?
            .map(Into::into))
    }

    /// Register a new lightning address.
    pub async fn register(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> WasmResult<LightningAddressInfo> {
        Ok(self
            .sdk
            .register_lightning_address(request.into())
            .await?
            .into())
    }

    /// Check if a username is available for lightning address registration.
    #[wasm_bindgen(js_name = "isAvailable")]
    pub async fn is_available(&self, username: &str) -> WasmResult<bool> {
        let request = breez_sdk_spark::CheckLightningAddressRequest {
            username: username.to_string(),
        };
        Ok(self
            .sdk
            .check_lightning_address_available(request)
            .await?)
    }

    /// Delete the currently registered lightning address.
    pub async fn delete(&self) -> WasmResult<()> {
        Ok(self.sdk.delete_lightning_address().await?)
    }
}
