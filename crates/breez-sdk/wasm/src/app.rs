use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::{AppConfig, ConnectConfig, WalletConfig},
    sdk::Wallet,
    sdk_builder::SdkBuilder,
};

/// Breez SDK entry point — holds validated, immutable configuration.
///
/// # Quick Start (single wallet)
///
/// ```js
/// const wallet = await Breez.connect({
///   apiKey: "brz_test_...",
///   network: "mainnet",
///   seed: { type: "mnemonic", mnemonic: "..." },
/// });
/// ```
///
/// # Advanced (multi-wallet)
///
/// ```js
/// const breez = new Breez({ apiKey: "brz_test_...", network: "mainnet" });
/// const wallet1 = await breez.connectWallet({ seed: seed1 });
/// const wallet2 = await breez.connectWallet({ seed: seed2 });
/// ```
#[wasm_bindgen(js_name = "Breez")]
pub struct Breez {
    inner: breez_sdk_spark::App,
}

#[wasm_bindgen(js_class = "Breez")]
impl Breez {
    /// Create a new `Breez` instance with the given configuration.
    ///
    /// Resolves all optional fields to sensible defaults. Validates required fields.
    #[wasm_bindgen(constructor)]
    pub fn new(config: AppConfig) -> WasmResult<Breez> {
        let core_config: breez_sdk_spark::AppConfig = config.into();
        let inner = breez_sdk_spark::App::new(core_config)?;
        Ok(Breez { inner })
    }

    /// Connect a wallet using this instance's configuration.
    ///
    /// For the common single-wallet case, use the static `Breez.connect()` instead.
    #[wasm_bindgen(js_name = "connectWallet")]
    pub async fn connect_wallet(&self, wallet_config: WalletConfig) -> WasmResult<Wallet> {
        let core_wallet_config: breez_sdk_spark::WalletConfig = wallet_config.into();

        let config = self.inner.to_config(&core_wallet_config);
        let storage_dir = self.inner.derive_storage_dir(&core_wallet_config)?;

        let builder = SdkBuilder::new(config.into(), core_wallet_config.seed.into())
            .with_default_storage(storage_dir)
            .await?;
        let sdk = builder.build().await?;
        Ok(sdk)
    }

    /// Single-step wallet connection for the common case.
    ///
    /// Combines configuration and wallet setup into one call:
    ///
    /// ```js
    /// const wallet = await Breez.connect({
    ///   apiKey: "brz_test_...",
    ///   network: "mainnet",
    ///   seed: { type: "mnemonic", mnemonic: "..." },
    /// });
    /// ```
    pub async fn connect(config: ConnectConfig) -> WasmResult<Wallet> {
        let core_config: breez_sdk_spark::ConnectConfig = config.into();
        let (app_config, wallet_config) = core_config.into_parts();

        let app = breez_sdk_spark::App::new(app_config)?;
        let merged_config = app.to_config(&wallet_config);
        let storage_dir = app.derive_storage_dir(&wallet_config)?;

        let builder = SdkBuilder::new(merged_config.into(), wallet_config.seed.into())
            .with_default_storage(storage_dir)
            .await?;
        let sdk = builder.build().await?;
        Ok(sdk)
    }
}

/// @deprecated Use `new Breez(config)` instead.
#[wasm_bindgen(js_name = "initializeApp")]
pub fn initialize_app(config: AppConfig) -> WasmResult<Breez> {
    Breez::new(config)
}

/// Backward-compat alias so existing `import { App }` still works.
#[wasm_bindgen(typescript_custom_section)]
const APP_ALIAS: &'static str = r#"
/** @deprecated Use `Breez` instead. */
export type App = Breez;"#;
