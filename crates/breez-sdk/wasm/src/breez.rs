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

    /// Connects to the Spark network using the provided configuration and seed.
    #[wasm_bindgen(js_name = "connect")]
    pub async fn connect(request: ConnectRequest) -> WasmResult<BreezSdk> {
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

    /// Verifies a signed message against a public key.
    ///
    /// This is a pure cryptographic operation that does not require a wallet
    /// connection. The message is SHA256 hashed before verification.
    #[wasm_bindgen(js_name = "verifyMessage")]
    pub fn verify_message(request: CheckMessageRequest) -> WasmResult<CheckMessageResponse> {
        Ok(breez_sdk_spark::verify_message(request.into())?.into())
    }
}
