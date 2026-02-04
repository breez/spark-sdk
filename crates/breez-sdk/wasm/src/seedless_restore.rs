use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::models::Seed;
use crate::models::passkey_prf_provider::{PasskeyPrfProvider, WasmPasskeyPrfProvider};

#[macros::extern_wasm_bindgen(breez_sdk_spark::seedless_restore::NostrRelayConfig)]
pub struct NostrRelayConfig {
    /// Optional Breez API key for authenticated access to the Breez relay.
    /// When provided, the Breez relay is added and NIP-42 authentication is enabled.
    pub breez_api_key: Option<String>,
    /// Connection timeout in seconds (default: 30)
    pub timeout_secs: u32,
}

/// WASM wrapper for SeedlessRestore.
///
/// Orchestrates seedless wallet creation and restore operations using
/// passkey PRF and Nostr relays.
#[wasm_bindgen]
pub struct SeedlessRestore {
    inner: breez_sdk_spark::seedless_restore::SeedlessRestore,
}

#[wasm_bindgen]
impl SeedlessRestore {
    /// Create a new SeedlessRestore instance.
    ///
    /// @param prf_provider - Platform implementation of passkey PRF operations
    /// @param relay_config - Configuration for Nostr relay connections
    #[wasm_bindgen(constructor)]
    pub fn new(prf_provider: PasskeyPrfProvider, relay_config: Option<NostrRelayConfig>) -> Self {
        let wasm_provider = WasmPasskeyPrfProvider {
            inner: prf_provider,
        };
        let inner = breez_sdk_spark::seedless_restore::SeedlessRestore::new(
            Arc::new(wasm_provider),
            relay_config.map(|rc| rc.into()),
        );
        Self { inner }
    }

    /// Create a new wallet seed from a user-provided salt.
    ///
    /// This method:
    /// 1. Derives the Nostr identity from the passkey using the magic salt
    /// 2. Checks if the salt already exists on Nostr (idempotency)
    /// 3. If not, publishes the salt to Nostr relays
    /// 4. Derives the wallet seed from the passkey using the user's salt
    ///
    /// @param salt - A user-chosen salt string (e.g., "personal", "business")
    /// @returns A Promise resolving to the derived wallet seed (24-word mnemonic)
    #[wasm_bindgen(js_name = "createSeed")]
    pub async fn create_seed(&self, salt: String) -> Result<JsValue, JsValue> {
        let seed = self
            .inner
            .create_seed(salt)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        let wasm_seed: Seed = seed.into();
        serde_wasm_bindgen::to_value(&wasm_seed).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// List all salts published to Nostr for this passkey's identity.
    ///
    /// This method queries Nostr relays for all kind-1 text note events
    /// authored by the Nostr identity derived from the passkey.
    ///
    /// @returns A Promise resolving to an array of salt strings
    #[wasm_bindgen(js_name = "listSalts")]
    pub async fn list_salts(&self) -> Result<JsValue, JsValue> {
        let salts = self
            .inner
            .list_salts()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&salts).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Restore a wallet seed from a specific salt.
    ///
    /// Use this after calling listSalts() to restore a specific wallet.
    /// This method only derives the seed; it does not publish anything.
    ///
    /// @param salt - The salt string to use for seed derivation
    /// @returns A Promise resolving to the derived wallet seed (24-word mnemonic)
    #[wasm_bindgen(js_name = "restoreSeed")]
    pub async fn restore_seed(&self, salt: String) -> Result<JsValue, JsValue> {
        let seed = self
            .inner
            .restore_seed(salt)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        let wasm_seed: Seed = seed.into();
        serde_wasm_bindgen::to_value(&wasm_seed).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Check if passkey PRF is available on this device.
    ///
    /// @returns A Promise resolving to true if PRF-capable passkey is available
    #[wasm_bindgen(js_name = "isPrfAvailable")]
    pub async fn is_prf_available(&self) -> Result<bool, JsValue> {
        self.inner
            .is_prf_available()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
}
