use anyhow::Result;
use breez_sdk_spark::*;

// ANCHOR: connect-simple
pub(crate) async fn connect_simple() -> Result<BreezSdk> {
    let sdk = Breez::connect(
        SdkCredentials::Mnemonic {
            api_key: "<breez api key>".to_string(),
            mnemonic: "<mnemonic words>".to_string(),
            passphrase: None,
        },
        None,
    )
    .await?;
    Ok(sdk)
}
// ANCHOR_END: connect-simple

// ANCHOR: connect-with-options
pub(crate) async fn connect_with_options() -> Result<BreezSdk> {
    let sdk = Breez::connect(
        SdkCredentials::Mnemonic {
            api_key: "<breez api key>".to_string(),
            mnemonic: "<mnemonic words>".to_string(),
            passphrase: None,
        },
        Some(ConnectOptions {
            network: Some(Network::Regtest),
            storage_dir: Some("./.data".to_string()),
            ..Default::default()
        }),
    )
    .await?;
    Ok(sdk)
}
// ANCHOR_END: connect-with-options

// ANCHOR: connect-with-providers
pub(crate) async fn connect_with_providers() -> Result<BreezSdk> {
    let providers = Providers {
        // storage: Some(my_custom_storage),
        // chain_service: Some(my_custom_chain_service),
        ..Default::default()
    };

    let sdk = Breez::with_providers(providers)
        .connect(
            SdkCredentials::Mnemonic {
                api_key: "<breez api key>".to_string(),
                mnemonic: "<mnemonic words>".to_string(),
                passphrase: None,
            },
            None,
        )
        .await?;
    Ok(sdk)
}
// ANCHOR_END: connect-with-providers
