use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        Seed,
        passkey_prf_provider::{PrfProvider, WasmPrfProvider},
    },
};

/// Relying Party and user identity for the built-in provider on the
/// zero-config path (ignored when you inject your own provider).
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::PasskeyProviderOptions)]
pub struct PasskeyProviderOptions {
    /// Relying Party ID. Unset uses the Breez shared RP.
    pub rp_id: Option<String>,
    /// Relying Party name. Unset uses the SDK default (`"Breez"`).
    pub rp_name: Option<String>,
    /// `user.name`: the account identifier the picker shows beneath the
    /// display name (e.g. `john@doe.com`). Unset uses `rpName`.
    pub user_name: Option<String>,
    /// `user.displayName`: the human-friendly name shown most
    /// prominently (e.g. `John Doe`). Unset uses `userName`.
    pub user_display_name: Option<String>,
}

/// Configuration for `PasskeyClient`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::PasskeyConfig)]
pub struct PasskeyConfig {
    /// Wallet label for `register` / `signIn` when no label is given.
    /// Unset falls back to the internal default `"Default"`.
    pub default_label: Option<String>,
    /// Relying Party and user identity for the built-in provider on the
    /// zero-config path.
    pub provider_options: Option<PasskeyProviderOptions>,
}

/// One-shot capability + configuration probe returned by
/// `PasskeyClient.checkAvailability`. Collapses `isSupported` +
/// `checkDomainAssociation` into one tagged value hosts branch on.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::PasskeyAvailability)]
pub enum PasskeyAvailability {
    Available,
    PrfUnsupported,
    NotAssociated { source: String, reason: String },
    Skipped { reason: String },
}

/// A wallet derived from a passkey.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::Wallet)]
pub struct Wallet {
    /// The derived seed.
    pub seed: Seed,
    /// The label used for derivation.
    pub label: String,
}

/// A passkey credential from `register` or `signIn`. `credentialId` is
/// always set. The attestation fields (`userId`, `aaguid`,
/// `backupEligible`) are populated on registration and null on sign-in
/// (an assertion carries no attestation). AAGUID is unverified: a
/// display hint only, never a trust signal.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::PasskeyCredential)]
pub struct PasskeyCredential {
    #[serde(with = "serde_bytes")]
    #[tsify(type = "Uint8Array")]
    pub credential_id: Vec<u8>,
    #[serde(with = "serde_bytes")]
    #[tsify(type = "Uint8Array")]
    pub user_id: Option<Vec<u8>>,
    #[serde(with = "serde_bytes")]
    #[tsify(type = "Uint8Array")]
    pub aaguid: Option<Vec<u8>>,
    pub backup_eligible: Option<bool>,
}

/// Request shape for `PasskeyClient.register`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisterRequest)]
pub struct RegisterRequest {
    pub label: Option<String>,
    #[tsify(type = "Uint8Array[]")]
    pub exclude_credentials: Option<Vec<Vec<u8>>>,
}

/// Response shape for `PasskeyClient.register`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisterResponse)]
pub struct RegisterResponse {
    pub wallet: Wallet,
    pub credential: Option<PasskeyCredential>,
}

/// Request shape for `PasskeyClient.signIn`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::SignInRequest)]
pub struct SignInRequest {
    pub label: Option<String>,
    #[tsify(type = "Uint8Array[]")]
    pub allow_credentials: Option<Vec<Vec<u8>>>,
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Response shape for `PasskeyClient.signIn`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::SignInResponse)]
pub struct SignInResponse {
    pub wallet: Wallet,
    pub labels: Vec<String>,
    pub credential: Option<PasskeyCredential>,
}

/// High-level orchestrator that collapses register / sign-in flows
/// into single calls. See the matching Rust types for full semantics;
/// the JS surface is a thin wasm-bindgen wrapper.
#[wasm_bindgen]
pub struct PasskeyClient {
    inner: breez_sdk_spark::passkey::PasskeyClient,
}

#[wasm_bindgen]
impl PasskeyClient {
    /// Create a `PasskeyClient` backed by the supplied `PrfProvider` and
    /// the default Nostr-backed label store. `breezApiKey` enables
    /// authenticated (NIP-42) relay access for label storage; omit for
    /// public relays only.
    #[wasm_bindgen(constructor)]
    pub fn new(
        prf_provider: PrfProvider,
        breez_api_key: Option<String>,
        config: Option<PasskeyConfig>,
    ) -> Self {
        let wasm_provider = WasmPrfProvider::new(prf_provider);
        Self {
            inner: breez_sdk_spark::passkey::PasskeyClient::new(
                Arc::new(wasm_provider),
                breez_api_key,
                config.map(Into::into),
            ),
        }
    }

    /// One-shot capability probe (PRF support + domain association)
    /// hosts can gate UX on.
    #[wasm_bindgen(js_name = "checkAvailability")]
    pub async fn check_availability(&self) -> WasmResult<PasskeyAvailability> {
        Ok(self.inner.check_availability().await?.into())
    }

    /// First-time setup. Drives the platform's create-passkey ceremony
    /// then derives the wallet seed in the same PRF assertion ceremony
    /// where the platform supports it.
    #[wasm_bindgen(js_name = "register")]
    pub async fn register(&self, request: RegisterRequest) -> WasmResult<RegisterResponse> {
        Ok(self.inner.register(request.into()).await?.into())
    }

    /// Returning-user sign-in. With `label` set, uses the fast path
    /// (one ceremony, no Nostr round-trip). With `label` omitted,
    /// derives the default-label wallet and discovers the user's
    /// label set in the same ceremony.
    #[wasm_bindgen(js_name = "signIn")]
    pub async fn sign_in(&self, request: SignInRequest) -> WasmResult<SignInResponse> {
        Ok(self.inner.sign_in(request.into()).await?.into())
    }

    /// Label sub-object. List / publish labels for this passkey's identity.
    #[wasm_bindgen(js_name = "labels")]
    pub fn labels(&self) -> PasskeyLabels {
        PasskeyLabels {
            inner: self.inner.labels(),
        }
    }
}

/// Label sub-object surfaced from `PasskeyClient.labels()`.
#[wasm_bindgen]
pub struct PasskeyLabels {
    inner: Arc<breez_sdk_spark::passkey::PasskeyLabels>,
}

#[wasm_bindgen]
impl PasskeyLabels {
    /// List labels published for this passkey's identity.
    #[wasm_bindgen(js_name = "list")]
    pub async fn list(&self) -> WasmResult<Vec<String>> {
        Ok(self.inner.list().await?)
    }

    /// Idempotently publish `label` for this passkey's identity.
    #[wasm_bindgen(js_name = "store")]
    pub async fn store(&self, label: String) -> WasmResult<()> {
        Ok(self.inner.store(label).await?)
    }
}
