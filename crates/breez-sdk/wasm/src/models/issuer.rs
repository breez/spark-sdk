#[macros::extern_wasm_bindgen(breez_sdk_spark::CreateIssuerTokenRequest)]
pub struct CreateIssuerTokenRequest {
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub is_freezable: bool,
    pub max_supply: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::MintIssuerTokenRequest)]
pub struct MintIssuerTokenRequest {
    pub amount: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::BurnIssuerTokenRequest)]
pub struct BurnIssuerTokenRequest {
    pub amount: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::FreezeIssuerTokenRequest)]
pub struct FreezeIssuerTokenRequest {
    pub address: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::FreezeIssuerTokenResponse)]
pub struct FreezeIssuerTokenResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::UnfreezeIssuerTokenRequest)]
pub struct UnfreezeIssuerTokenRequest {
    pub address: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::UnfreezeIssuerTokenResponse)]
pub struct UnfreezeIssuerTokenResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}
