use wasm_bindgen::prelude::*;

use crate::{error::WasmResult, models::*, sdk::BreezSdk, sdk_builder::SdkBuilder};

/// Top-level namespace for the Breez SDK.
///
/// Groups all static/global SDK functions that don't require a wallet
/// connection. Use `Breez.connect()` to obtain a `BreezSdk` instance.
#[wasm_bindgen]
pub struct Breez;

#[wasm_bindgen]
impl Breez {
    /// Returns a default SDK configuration for the given network.
    #[wasm_bindgen(js_name = "defaultConfig")]
    pub fn default_config(network: Network) -> Config {
        #[allow(deprecated)]
        breez_sdk_spark::default_config(network.into()).into()
    }

    /// Parses a payment input string and returns the identified type.
    ///
    /// Supports BOLT11 invoices, Lightning addresses, LNURL variants, Bitcoin
    /// addresses, Spark addresses/invoices, BIP21 URIs, and more.
    #[wasm_bindgen(js_name = "parse")]
    pub async fn parse(input: &str) -> WasmResult<InputType> {
        #[allow(deprecated)]
        let result = breez_sdk_spark::parse_input(input, None).await?;
        Ok(result.into())
    }

    /// Connects to the Spark network using credentials and optional configuration.
    ///
    /// This is the primary entry point for initializing the SDK. For most use cases,
    /// only credentials are needed — sensible defaults are applied automatically.
    #[wasm_bindgen(js_name = "connect")]
    pub async fn connect(
        credentials: SdkCredentials,
        options: Option<ConnectOptions>,
    ) -> WasmResult<BreezSdk> {
        let credentials_core: breez_sdk_spark::SdkCredentials = credentials.into();
        let opts: breez_sdk_spark::ConnectOptions = options.map(Into::into).unwrap_or_default();
        let (config, seed) = credentials_core.to_config_and_seed(&opts)?;
        let storage_dir = opts
            .storage_dir
            .clone()
            .unwrap_or_else(|| "./.data".to_string());

        let wasm_config: Config = config.into();
        let wasm_seed: Seed = seed.into();
        let mut builder = SdkBuilder::new(wasm_config, wasm_seed)
            .with_default_storage(storage_dir)
            .await?;

        if let Some(key_set) = opts.key_set {
            let wasm_key_set: KeySetConfig = key_set.into();
            builder = builder.with_key_set(wasm_key_set);
        }

        let sdk = builder.build().await?;
        Ok(sdk)
    }

    /// Connects using a legacy `ConnectRequest`.
    ///
    /// Prefer `Breez.connect(credentials, options)` for new code.
    #[wasm_bindgen(js_name = "connectLegacy")]
    pub async fn connect_legacy(request: ConnectRequest) -> WasmResult<BreezSdk> {
        let builder = SdkBuilder::new(request.config, request.seed)
            .with_default_storage(request.storage_dir)
            .await?;
        let sdk = builder.build().await?;
        Ok(sdk)
    }

    /// Connects to the Spark network using an external signer.
    #[wasm_bindgen(js_name = "connectWithSigner")]
    pub async fn connect_with_signer(
        config: Config,
        signer: crate::signer::JsExternalSigner,
        storage_dir: String,
    ) -> WasmResult<BreezSdk> {
        let builder = SdkBuilder::new_with_signer(config, signer)
            .with_default_storage(storage_dir)
            .await?;
        let sdk = builder.build().await?;
        Ok(sdk)
    }

    /// Creates a default external signer from a mnemonic phrase.
    #[wasm_bindgen(js_name = "defaultExternalSigner")]
    pub fn default_external_signer(
        mnemonic: String,
        passphrase: Option<String>,
        network: Network,
        key_set_config: Option<KeySetConfig>,
    ) -> WasmResult<crate::signer::DefaultSigner> {
        #[allow(deprecated)]
        let signer = breez_sdk_spark::default_external_signer(
            mnemonic,
            passphrase,
            network.into(),
            key_set_config.map(|k| k.into()),
        )?;

        Ok(crate::signer::DefaultSigner::new(signer))
    }

    /// Fetches the current status of Spark network services.
    #[wasm_bindgen(js_name = "getSparkStatus")]
    pub async fn get_spark_status() -> WasmResult<SparkStatus> {
        #[allow(deprecated)]
        let status = breez_sdk_spark::get_spark_status().await?;
        Ok(status.into())
    }
}
