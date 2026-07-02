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

/// Provisions a Turnkey wallet once (at user creation) and returns opaque bytes
/// to persist. Pass them back to `createTurnkeySigner` on later inits to build
/// the signer with no network calls. Store them encrypted: they hold a scoped
/// ECIES/HMAC key (never funds or the Spark identity).
#[wasm_bindgen(js_name = "provisionTurnkeySigner")]
pub async fn provision_turnkey_signer(config: TurnkeyConfig) -> WasmResult<Vec<u8>> {
    Ok(
        breez_sdk_spark::turnkey::provision_turnkey_signer(config.into())
            .await?
            .bytes,
    )
}

/// Builds the Turnkey-backed signers from `config`, then pass
/// `signers.breezSigner` and `signers.sparkSigner` to `connectWithSigner`,
/// exactly as with any other external signer.
///
/// Pass `provisioned` (from `provisionTurnkeySigner`) to build with no network
/// calls; omit it to provision lazily on first use. A `provisioned` blob that no
/// longer matches `config` throws, signalling a re-provision.
#[wasm_bindgen(js_name = "createTurnkeySigner")]
pub async fn create_turnkey_signer(
    config: TurnkeyConfig,
    provisioned: Option<Vec<u8>>,
) -> WasmResult<crate::sdk::ExternalSigners> {
    let provisioned =
        provisioned.map(|bytes| breez_sdk_spark::turnkey::TurnkeyProvisionedSigner { bytes });
    let signers =
        breez_sdk_spark::turnkey::create_turnkey_signer(config.into(), provisioned).await?;
    Ok(crate::sdk::ExternalSigners::new(
        ExternalBreezSignerHandle::new(signers.breez_signer),
        ExternalSparkSignerHandle::new(signers.spark_signer),
    ))
}
