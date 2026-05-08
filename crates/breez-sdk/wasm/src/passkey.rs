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
    /// Connection timeout in seconds. Defaults to 30 when `None`.
    pub timeout_secs: Option<u32>,
}

/// A wallet derived from a passkey.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::Wallet)]
pub struct Wallet {
    /// The derived seed.
    pub seed: Seed,
    /// The label used for derivation.
    pub label: String,
}

/// Caller-supplied salt for `setupWallet({ extraSalts })`. Yields a
/// 32-byte output keyed by `name` in the `WalletSetup.extraSeeds` map.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::NamedSalt)]
pub struct NamedSalt {
    pub name: String,
}

/// Request shape for `Passkey.setupWallet`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::SetupWalletRequest)]
pub struct SetupWalletRequest {
    pub label: Option<String>,
    pub publish_label: bool,
    pub extra_salts: Vec<NamedSalt>,
}

/// Response shape for `Passkey.setupWallet`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::WalletSetup)]
pub struct WalletSetup {
    pub wallet: Wallet,
    pub extra_seeds: HashMap<String, Vec<u8>>,
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

/// Passkey-based wallet operations using WebAuthn PRF extension.
///
/// Wraps a `PrfProvider` and optional relay configuration to provide
/// wallet derivation and label management via Nostr relays.
#[wasm_bindgen]
pub struct Passkey {
    inner: breez_sdk_spark::passkey::Passkey,
}

#[wasm_bindgen]
impl Passkey {
    /// Create a new `Passkey` instance.
    ///
    /// @param prfProvider - Implementation of PRF operations (typically the
    ///                      built-in `PasskeyProvider`, or a custom `PrfProvider`)
    /// @param relayConfig - Optional configuration for Nostr relay connections
    #[wasm_bindgen(constructor)]
    pub fn new(prf_provider: PrfProvider, relay_config: Option<NostrRelayConfig>) -> Self {
        let wasm_provider = WasmPrfProvider::new(prf_provider);
        Self {
            inner: breez_sdk_spark::passkey::Passkey::new(
                Arc::new(wasm_provider),
                relay_config.map(|rc| rc.into()),
            ),
        }
    }

    /// Derive a wallet for a given label.
    ///
    /// Uses the passkey PRF to derive a `Wallet` containing the seed and resolved label.
    ///
    /// @param label - Optional label string (defaults to "Default")
    #[wasm_bindgen(js_name = "getWallet")]
    pub async fn get_wallet(&self, label: Option<String>) -> WasmResult<Wallet> {
        Ok(self.inner.get_wallet(label).await?.into())
    }

    /// Single-prompt setup. See the `SetupWalletRequest` /
    /// `WalletSetup` shape for full semantics. ⌈N / 2⌉ prompts where
    /// the authenticator supports `prf.eval.first` + `.second`, N
    /// otherwise. Pass `publishLabel: false` for speculative
    /// cold-restore. Pass `extraSalts` to derive caller-named seeds in
    /// the same ceremony.
    #[wasm_bindgen(js_name = "setupWallet")]
    pub async fn setup_wallet(&self, request: SetupWalletRequest) -> WasmResult<WalletSetup> {
        Ok(self.inner.setup_wallet(request.into()).await?.into())
    }

    /// List all labels published to Nostr for this passkey's identity.
    ///
    /// Requires 1 PRF call (for Nostr identity derivation).
    #[wasm_bindgen(js_name = "listLabels")]
    pub async fn list_labels(&self) -> WasmResult<Vec<String>> {
        Ok(self.inner.list_labels().await?)
    }

    /// Publish a label to Nostr relays for this passkey's identity.
    ///
    /// Idempotent: if the label already exists, it is not published again.
    /// Requires 1 PRF call.
    #[wasm_bindgen(js_name = "storeLabel")]
    pub async fn store_label(&self, label: String) -> WasmResult<()> {
        Ok(self.inner.store_label(label).await?)
    }

    /// Check if passkey PRF is available on this device.
    #[wasm_bindgen(js_name = "isAvailable")]
    pub async fn is_available(&self) -> WasmResult<bool> {
        Ok(self.inner.is_available().await?)
    }
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
