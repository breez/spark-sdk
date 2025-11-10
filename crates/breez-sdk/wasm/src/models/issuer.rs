#[macros::extern_wasm_bindgen(breez_sdk_spark::CreateTokenRequest)]
pub struct CreateTokenRequest {
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub is_freezable: bool,
    pub max_supply: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::GetIssuerTokenBalanceResponse)]
pub struct GetIssuerTokenBalanceResponse {
    pub identifier: String,
    pub balance: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::MintTokensRequest)]
pub struct MintTokensRequest {
    pub amount: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::BurnTokensRequest)]
pub struct BurnTokensRequest {
    pub amount: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::FreezeTokensRequest)]
pub struct FreezeTokensRequest {
    pub address: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::FreezeTokensResponse)]
pub struct FreezeTokensResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::UnfreezeTokensRequest)]
pub struct UnfreezeTokensRequest {
    pub address: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::UnfreezeTokensResponse)]
pub struct UnfreezeTokensResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}
