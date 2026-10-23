use super::{ConversionOptions, Payment};

#[macros::extern_wasm_bindgen(breez_sdk_spark::PayX402Request)]
pub struct PayX402Request {
    pub payment_required: String,
    pub request: X402Request,
    pub max_amount: X402PaymentAmount,
    pub conversion_options: Option<ConversionOptions>,
    pub completion_timeout_secs: Option<u32>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::X402Request)]
pub enum X402Request {
    Http {
        method: String,
        url: String,
        headers: Vec<X402Header>,
        body: Option<Vec<u8>>,
    },
    Mcp {
        server: String,
        params: String,
    },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::X402PaymentAmount)]
pub enum X402PaymentAmount {
    Lightning { amount_sats: u64 },
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::X402Header)]
pub struct X402Header {
    pub name: String,
    pub value: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::X402PaymentStatus)]
pub enum X402PaymentStatus {
    Paid,
    Pending,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::PayX402Response)]
pub struct PayX402Response {
    pub status: X402PaymentStatus,
    pub payment_signature: Option<String>,
    pub payment: Payment,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::CreateX402ChallengeRequest)]
pub struct CreateX402ChallengeRequest {
    pub request: X402Request,
    pub bound_names: Vec<String>,
    pub amounts: Vec<X402PaymentAmount>,
    pub max_timeout_secs: Option<u32>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::CreateX402ChallengeResponse)]
pub struct CreateX402ChallengeResponse {
    pub payment_required: String,
    pub payment_hash: String,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SettleX402Request)]
pub struct SettleX402Request {
    pub payment: String,
    pub request: X402Request,
    pub bound_names: Vec<String>,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::SettleX402Response)]
pub struct SettleX402Response {
    pub success: bool,
    pub error_reason: Option<String>,
    pub payment_response: String,
    pub payment_hash: Option<String>,
    pub amount: Option<X402PaymentAmount>,
}
