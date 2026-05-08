use std::collections::HashMap;
use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        Seed,
        passkey_prf_provider::{PrfProvider, WasmPrfProvider},
    },
};

/// Nostr relay configuration for passkey label operations.
///
/// Used by `Passkey.listLabels` and `Passkey.storeLabel`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::NostrRelayConfig)]
pub struct NostrRelayConfig {
    /// Optional Breez API key for authenticated access to the Breez relay.
    /// When provided, the Breez relay is added and NIP-42 authentication is enabled.
    pub breez_api_key: Option<String>,
}

/// A wallet derived from a passkey.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::Wallet)]
pub struct Wallet {
    /// The derived seed.
    pub seed: Seed,
    /// The label used for derivation.
    pub label: String,
}

/// Caller-supplied salt for `extraSalts` on `PasskeyClient.register`,
/// `restore`, and `derive`. Yields a 32-byte output keyed by `name` in
/// the response's `extraSeeds` map.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::NamedSalt)]
pub struct NamedSalt {
    pub name: String,
}

/// Authenticator metadata returned by `PasskeyClient.register`.
/// `aaguid` (16-byte authenticator identifier) and `backupEligible`
/// (BE flag) are best-effort: platforms that don't expose
/// authenticator data leave them `null`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisteredCredential)]
pub struct RegisteredCredential {
    pub credential_id: Vec<u8>,
    pub aaguid: Option<Vec<u8>>,
    pub backup_eligible: Option<bool>,
}

/// Request shape for `PasskeyClient.register`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisterRequest)]
pub struct RegisterRequest {
    pub label: Option<String>,
    pub extra_salts: Vec<NamedSalt>,
    pub exclude_credential_ids: Vec<Vec<u8>>,
    pub user_id: Option<Vec<u8>>,
    pub user_name: Option<String>,
    pub user_display_name: Option<String>,
}

/// Response shape for `PasskeyClient.register`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisterResponse)]
pub struct RegisterResponse {
    pub wallet: Wallet,
    pub credential: RegisteredCredential,
    pub extra_seeds: HashMap<String, Vec<u8>>,
}

/// Request shape for `PasskeyClient.restore`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RestoreRequest)]
pub struct RestoreRequest {
    pub candidate_label: Option<String>,
    pub extra_salts: Vec<NamedSalt>,
}

/// Response shape for `PasskeyClient.restore`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RestoreResponse)]
pub struct RestoreResponse {
    pub wallet: Wallet,
    pub candidate_matched: bool,
    pub labels: Vec<String>,
    pub extra_seeds: HashMap<String, Vec<u8>>,
}

/// Request shape for `PasskeyClient.derive`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::DeriveRequest)]
pub struct DeriveRequest {
    pub label: Option<String>,
    pub extra_salts: Vec<NamedSalt>,
}

/// Response shape for `PasskeyClient.derive`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::DeriveResponse)]
pub struct DeriveResponse {
    pub wallet: Wallet,
    pub extra_seeds: HashMap<String, Vec<u8>>,
}

/// High-level orchestrator that collapses register / restore / derive
/// flows into single calls. See the matching Rust types for full
/// semantics; the JS surface is a thin wasm-bindgen wrapper.
#[wasm_bindgen]
pub struct PasskeyClient {
    inner: breez_sdk_spark::passkey::PasskeyClient,
}

#[wasm_bindgen]
impl PasskeyClient {
    /// Create a `PasskeyClient` backed by the supplied `PrfProvider`
    /// and the default Nostr-backed label store.
    #[wasm_bindgen(constructor)]
    pub fn new(prf_provider: PrfProvider, relay_config: Option<NostrRelayConfig>) -> Self {
        let wasm_provider = WasmPrfProvider::new(prf_provider);
        Self {
            inner: breez_sdk_spark::passkey::PasskeyClient::new(
                Arc::new(wasm_provider),
                relay_config.map(|rc| rc.into()),
            ),
        }
    }

    /// First-time setup. Drives the platform's create-passkey ceremony
    /// then derives the wallet seed and any extra salts in the same
    /// PRF assertion ceremony where the platform supports it.
    #[wasm_bindgen(js_name = "register")]
    pub async fn register(&self, request: RegisterRequest) -> WasmResult<RegisterResponse> {
        Ok(self.inner.register(request.into()).await?.into())
    }

    /// Speculative cold-restore. Derives a wallet for
    /// `candidateLabel` without publishing it, then runs `listLabels`
    /// off the cached identity (no extra prompts). Pass-through
    /// failure of the label-store query leaves `labels` empty.
    #[wasm_bindgen(js_name = "restore")]
    pub async fn restore(&self, request: RestoreRequest) -> WasmResult<RestoreResponse> {
        Ok(self.inner.restore(request.into()).await?.into())
    }

    /// Returning user with the correct label cached locally.
    /// `publishLabel` is implicit `false`; if the label is not yet
    /// published, call `storeLabel` separately.
    #[wasm_bindgen(js_name = "derive")]
    pub async fn derive(&self, request: DeriveRequest) -> WasmResult<DeriveResponse> {
        Ok(self.inner.derive(request.into()).await?.into())
    }

    /// List labels published for this passkey's identity.
    #[wasm_bindgen(js_name = "listLabels")]
    pub async fn list_labels(&self) -> WasmResult<Vec<String>> {
        Ok(self.inner.list_labels().await?)
    }

    /// Idempotently publish `label` for this passkey's identity.
    #[wasm_bindgen(js_name = "storeLabel")]
    pub async fn store_label(&self, label: String) -> WasmResult<()> {
        Ok(self.inner.store_label(label).await?)
    }

    /// Pass-through to `Passkey.isAvailable`.
    #[wasm_bindgen(js_name = "isAvailable")]
    pub async fn is_available(&self) -> WasmResult<bool> {
        Ok(self.inner.is_available().await?)
    }
}
