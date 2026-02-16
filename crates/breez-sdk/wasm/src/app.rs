use wasm_bindgen::prelude::*;

use crate::{
    error::WasmResult,
    models::ClientConfig,
    sdk::BreezClient,
    sdk_builder::SdkBuilder,
};

/// Breez SDK entry point.
///
/// Provides a static [`connect`](Self::connect) method that takes a
/// [`ClientConfig`] and returns a connected [`BreezClient`].
///
/// # Quick Start
///
/// ```js
/// const client = await Breez.connect({
///   apiKey: "brz_test_...",
///   network: "mainnet",
///   seed: { type: "mnemonic", mnemonic: "..." },
/// });
/// ```
///
/// # Advanced (custom components)
///
/// ```js
/// const builder = await Breez.builder({
///   apiKey: "brz_test_...",
///   network: "mainnet",
///   seed: { type: "mnemonic", mnemonic: "..." },
/// });
/// builder.withStorage(myCustomStorage);
/// const client = await builder.build();
/// ```
#[wasm_bindgen(js_name = "Breez")]
pub struct Breez;

#[wasm_bindgen(js_class = "Breez")]
impl Breez {
    /// Connect to the Breez SDK.
    ///
    /// Validates the configuration, resolves defaults, auto-derives the
    /// storage directory from the seed fingerprint (if not provided),
    /// and initializes the client.
    ///
    /// ```js
    /// const client = await Breez.connect({
    ///   apiKey: "brz_test_...",
    ///   network: "mainnet",
    ///   seed: { type: "mnemonic", mnemonic: "..." },
    /// });
    /// ```
    pub async fn connect(config: ClientConfig) -> WasmResult<BreezClient> {
        let core_config: breez_sdk_spark::ClientConfig = config.into();
        let resolved = breez_sdk_spark::app::resolve_config(&core_config)?;
        let storage_dir = breez_sdk_spark::app::derive_storage_dir(&core_config)?;

        // Convert core types back to WASM wrapper types for SdkBuilder
        let wasm_config: crate::models::Config = resolved.into();
        let wasm_seed: crate::models::Seed = core_config.seed.into();

        let builder = SdkBuilder::new(wasm_config, wasm_seed)
            .with_default_storage(storage_dir)
            .await?;
        let sdk = builder.build().await?;
        Ok(sdk)
    }

    /// Create an [`SdkBuilder`] from a [`ClientConfig`].
    ///
    /// Use this when you need to customize low-level components (storage,
    /// chain service, fiat service, LNURL client, payment observer, key set)
    /// before connecting.
    ///
    /// The returned builder has the resolved config and default storage
    /// directory already configured. You can override individual components
    /// via the builder's fluent methods before calling `.build()`.
    ///
    /// ```js
    /// const builder = await Breez.builder({
    ///   apiKey: "brz_test_...",
    ///   network: "mainnet",
    ///   seed: { type: "mnemonic", mnemonic: "..." },
    /// });
    /// builder.withStorage(myCustomStorage);
    /// const client = await builder.build();
    /// ```
    pub async fn builder(config: ClientConfig) -> WasmResult<SdkBuilder> {
        let core_config: breez_sdk_spark::ClientConfig = config.into();
        let resolved = breez_sdk_spark::app::resolve_config(&core_config)?;
        let storage_dir = breez_sdk_spark::app::derive_storage_dir(&core_config)?;

        let wasm_config: crate::models::Config = resolved.into();
        let wasm_seed: crate::models::Seed = core_config.seed.into();

        Ok(SdkBuilder::new(wasm_config, wasm_seed)
            .with_default_storage(storage_dir)
            .await?)
    }
}
