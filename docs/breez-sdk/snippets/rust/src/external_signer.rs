use breez_sdk_spark::*;

// ANCHOR: default-external-signer
fn create_signers() -> Result<ExternalSigners, SdkError> {
    let mnemonic = "<mnemonic words>".to_string();
    let network = Network::Mainnet;

    let signers = default_external_signers(
        mnemonic,
        None, // passphrase
        network,
        Some(0), // account number
    )?;

    Ok(signers)
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
async fn connect_example(signers: ExternalSigners) -> Result<BreezSdk, SdkError> {
    // Create the config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Connect using the external signers
    let sdk = connect_with_signer(ConnectWithSignerRequest {
        config,
        breez_signer: signers.breez_signer,
        spark_signer: signers.spark_signer,
        supports_ecies_hmac: true,
        storage_dir: "./.data".to_string(),
    })
    .await?;

    Ok(sdk)
}
// ANCHOR_END: connect-with-signer
