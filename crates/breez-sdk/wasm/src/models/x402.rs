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
