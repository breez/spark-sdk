use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{CheckMessageRequest, CheckMessageResponse, InputType},
};

/// Parse any payment input string (invoice, address, LNURL, Lightning Address).
///
/// This is a standalone function — no wallet connection needed.
///
/// ```js
/// import { parseInput } from '@breeztech/breez-sdk-spark';
/// const input = await parseInput("lnbc1...");
/// ```
#[wasm_bindgen(js_name = "parseInput")]
pub async fn parse_input(input: &str) -> WasmResult<InputType> {
    Ok(breez_sdk_spark::parse_input(input, None).await?.into())
}

/// Verify a message signature.
///
/// This is a standalone function — no wallet connection needed.
/// Uses pure secp256k1 ECDSA verification.
///
/// ```js
/// import { verifyMessage } from '@breeztech/breez-sdk-spark';
/// const { isValid } = verifyMessage({ message, pubkey, signature });
/// ```
#[wasm_bindgen(js_name = "verifyMessage")]
pub fn verify_message(request: CheckMessageRequest) -> WasmResult<CheckMessageResponse> {
    Ok(breez_sdk_spark::verify_message(request.into())?.into())
}
