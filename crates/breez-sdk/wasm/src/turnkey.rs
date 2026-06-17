//! WASM bindings for the Turnkey signer backend.
//!
//! `createTurnkeySigner` builds the Rust Turnkey signers and returns them as
//! handles that satisfy the JS signer interfaces, so Turnkey is passed to the
//! ordinary `connectWithSigner` like any other signer.

use wasm_bindgen::prelude::*;

use crate::error::WasmResult;
use crate::models::Network;
use crate::signer::{ExternalBreezSignerHandle, ExternalSparkSignerHandle};

#[macros::extern_wasm_bindgen(breez_sdk_spark::turnkey::TurnkeyRetryConfig)]
pub struct TurnkeyRetryConfig {
    pub initial_delay_ms: u64,
    pub multiplier: f64,
    pub max_delay_ms: u64,
    pub max_retries: u32,
    pub request_timeout_ms: u64,
}

#[macros::extern_wasm_bindgen(breez_sdk_spark::turnkey::TurnkeyConfig)]
pub struct TurnkeyConfig {
    pub base_url: Option<String>,
    pub organization_id: String,
    pub api_public_key: String,
    pub api_private_key: String,
    pub wallet_id: String,
    pub network: Network,
    pub account_number: Option<u32>,
    pub retry: Option<TurnkeyRetryConfig>,
}

/// Builds the Turnkey-backed signers from `config`, then pass
/// `signers.breezSigner` and `signers.sparkSigner` to `connectWithSigner`,
/// exactly as with any other external signer.
#[wasm_bindgen(js_name = "createTurnkeySigner")]
pub async fn create_turnkey_signer(
    config: TurnkeyConfig,
) -> WasmResult<crate::sdk::ExternalSigners> {
    let signers = breez_sdk_spark::turnkey::create_turnkey_signer(config.into()).await?;
    Ok(crate::sdk::ExternalSigners::new(
        ExternalBreezSignerHandle::new(signers.breez_signer),
        ExternalSparkSignerHandle::new(signers.spark_signer),
    ))
}
