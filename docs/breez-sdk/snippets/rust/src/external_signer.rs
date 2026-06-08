use breez_sdk_spark::signer::{ExternalBreezSigner, ExternalSparkSigner};
use breez_sdk_spark::*;
use std::sync::Arc;

// ANCHOR: default-external-signer
fn create_signer() -> Result<Arc<dyn ExternalBreezSigner>, SdkError> {
    let mnemonic = "<mnemonic words>".to_string();
    let network = Network::Mainnet;

    let signer = default_external_signer(
        mnemonic,
        None, // passphrase
        network,
        Some(KeySetConfig {
            account_number: Some(0),
        }),
    )?;

    Ok(signer)
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
async fn connect_example(
    signer: Arc<dyn ExternalBreezSigner>,
    spark_signer: Arc<dyn ExternalSparkSigner>,
) -> Result<BreezSdk, SdkError> {
    // Create the config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Connect using the external signers
    let sdk = connect_with_signer(ConnectWithSignerRequest {
        config,
        signer,
        spark_signer,
        storage_dir: "./.data".to_string(),
    })
    .await?;

    Ok(sdk)
}
// ANCHOR_END: connect-with-signer
