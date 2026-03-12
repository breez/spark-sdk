use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        Seed,
        passkey_prf_provider::{PasskeyPrfProvider, WasmPasskeyPrfProvider},
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

/// Passkey-based wallet operations using WebAuthn PRF extension.
///
/// Wraps a `PasskeyPrfProvider` and optional relay configuration to provide
/// wallet derivation and label management via Nostr relays.
#[wasm_bindgen]
pub struct Passkey {
    inner: breez_sdk_spark::passkey::Passkey,
}

#[wasm_bindgen]
impl Passkey {
    /// Create a new `Passkey` instance.
    ///
    /// @param prfProvider - Platform implementation of passkey PRF operations
    /// @param relayConfig - Optional configuration for Nostr relay connections
    #[wasm_bindgen(constructor)]
    pub fn new(prf_provider: PasskeyPrfProvider, relay_config: Option<NostrRelayConfig>) -> Self {
        let wasm_provider = WasmPasskeyPrfProvider {
            inner: prf_provider,
        };
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
