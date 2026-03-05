use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{
        Seed,
        passkey_prf_provider::{PasskeyPrfProvider, WasmPasskeyPrfProvider},
    },
};

/// Nostr relay configuration for passkey wallet name operations.
///
/// Used by `Passkey.listWalletNames` and `Passkey.storeWalletName`.
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
    /// The wallet name used for derivation.
    pub name: String,
}

/// Passkey-based wallet operations using WebAuthn PRF extension.
///
/// Wraps a `PasskeyPrfProvider` and optional relay configuration to provide
/// wallet derivation and name management via Nostr relays.
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

    /// Derive a wallet for a given wallet name.
    ///
    /// Uses the passkey PRF to derive a `Wallet` containing the seed and resolved name.
    ///
    /// @param walletName - Optional wallet name string (defaults to "Default")
    #[wasm_bindgen(js_name = "getWallet")]
    pub async fn get_wallet(&self, wallet_name: Option<String>) -> WasmResult<Wallet> {
        Ok(self.inner.get_wallet(wallet_name).await?.into())
    }

    /// List all wallet names published to Nostr for this passkey's identity.
    ///
    /// Requires 1 PRF call (for Nostr identity derivation).
    #[wasm_bindgen(js_name = "listWalletNames")]
    pub async fn list_wallet_names(&self) -> WasmResult<Vec<String>> {
        Ok(self.inner.list_wallet_names().await?)
    }

    /// Publish a wallet name to Nostr relays for this passkey's identity.
    ///
    /// Idempotent: if the wallet name already exists, it is not published again.
    /// Requires 1 PRF call.
    #[wasm_bindgen(js_name = "storeWalletName")]
    pub async fn store_wallet_name(&self, wallet_name: String) -> WasmResult<()> {
        Ok(self.inner.store_wallet_name(wallet_name).await?)
    }

    /// Check if passkey PRF is available on this device.
    #[wasm_bindgen(js_name = "isAvailable")]
    pub async fn is_available(&self) -> WasmResult<bool> {
        Ok(self.inner.is_available().await?)
    }
}
