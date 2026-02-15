use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{UpdateUserSettingsRequest, UserSettings},
};

/// Sub-object for user settings.
///
/// Access via `wallet.settings`.
#[wasm_bindgen(js_name = "SettingsApi")]
pub struct SettingsApi {
    pub(crate) sdk: Rc<breez_sdk_spark::BreezSdk>,
}

#[wasm_bindgen(js_class = "SettingsApi")]
impl SettingsApi {
    /// Get the current user settings.
    pub async fn get(&self) -> WasmResult<UserSettings> {
        Ok(self.sdk.get_user_settings().await?.into())
    }

    /// Update user settings.
    pub async fn update(&self, request: UpdateUserSettingsRequest) -> WasmResult<()> {
        Ok(self.sdk.update_user_settings(request.into()).await?)
    }
}
