use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        Seed,
        passkey_prf_provider::{PrfProvider, WasmPrfProvider},
    },
};

/// Configuration for `PasskeyClient`. `rpId` / `rpName` configure the
/// built-in provider on the zero-config path (ignored when you inject
/// your own provider, which owns its RP).
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::PasskeyConfig)]
pub struct PasskeyConfig {
    /// Wallet label for `register` / `signIn` when no label is given.
    /// Unset falls back to the internal default `"Default"`.
    pub default_label: Option<String>,
    /// Relying Party ID for the built-in provider. Unset uses the Breez
    /// shared RP.
    pub rp_id: Option<String>,
    /// Relying Party name for the built-in provider. Unset uses the SDK
    /// default (`"Breez"`).
    pub rp_name: Option<String>,
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

/// Authenticator metadata returned by `PasskeyClient.register`. `userId`
/// is the provider-generated WebAuthn user handle (never host-supplied).
/// `aaguid` (provider identifier) and `backupEligible` are null when the
/// platform doesn't expose them. AAGUID is unverified attestation: a
/// display hint only, never a trust signal.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisteredCredential)]
pub struct RegisteredCredential {
    pub credential_id: Vec<u8>,
    pub user_id: Vec<u8>,
    pub aaguid: Option<Vec<u8>>,
    pub backup_eligible: Option<bool>,
}

/// Request shape for `PasskeyClient.register`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisterRequest)]
pub struct RegisterRequest {
    pub label: Option<String>,
    pub exclude_credentials: Vec<Vec<u8>>,
}

/// Response shape for `PasskeyClient.register`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::RegisterResponse)]
pub struct RegisterResponse {
    pub wallet: Wallet,
    pub credential: RegisteredCredential,
}

/// Request shape for `PasskeyClient.signIn`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::SignInRequest)]
pub struct SignInRequest {
    pub label: Option<String>,
    pub allow_credentials: Vec<Vec<u8>>,
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Response shape for `PasskeyClient.signIn`.
#[macros::extern_wasm_bindgen(breez_sdk_spark::passkey::SignInResponse)]
pub struct SignInResponse {
    pub wallet: Wallet,
    pub labels: Vec<String>,
    pub credential_id: Option<Vec<u8>>,
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

    /// Credential sub-object. Inspect / mutate the provider's
    /// persisted credential-ID set.
    #[wasm_bindgen(js_name = "credentials")]
    pub fn credentials(&self) -> PasskeyCredentials {
        PasskeyCredentials {
            inner: self.inner.credentials(),
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

/// Credential sub-object surfaced from `PasskeyClient.credentials()`.
/// Reads / mutates the provider's persisted credential-ID set.
#[wasm_bindgen]
pub struct PasskeyCredentials {
    inner: Arc<breez_sdk_spark::passkey::PasskeyCredentials>,
}

#[wasm_bindgen]
impl PasskeyCredentials {
    /// Read the persisted set of credential IDs for the current RP.
    #[wasm_bindgen(js_name = "get")]
    pub async fn get(&self) -> WasmResult<Vec<js_sys::Uint8Array>> {
        let ids = self.inner.get().await?;
        Ok(ids
            .into_iter()
            .map(|id| js_sys::Uint8Array::from(id.as_slice()))
            .collect())
    }

    /// Drop a single credential ID from the persisted set.
    #[wasm_bindgen(js_name = "remove")]
    pub async fn remove(&self, credential_id: Vec<u8>) -> WasmResult<()> {
        Ok(self.inner.remove(credential_id).await?)
    }

    /// Clear the persisted credential-ID set for the current RP.
    #[wasm_bindgen(js_name = "clear")]
    pub async fn clear(&self) -> WasmResult<()> {
        Ok(self.inner.clear().await?)
    }
}
