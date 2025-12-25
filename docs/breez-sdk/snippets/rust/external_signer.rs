use breez_sdk_spark::*;

// ANCHOR: default-external-signer
fn create_signer() -> Result<Arc<dyn ExternalSigner>, SdkError> {
    let mnemonic = "<mnemonic words>".to_string();
    let network = Network::Mainnet;
    let key_set_type = KeySetType::Default;
    let use_address_index = false;
    let account_number = Some(0);
    
    let signer = default_external_signer(
        mnemonic,
        None, // passphrase
        network,
        key_set_type,
        use_address_index,
        account_number,
    )?;
    
    Ok(signer)
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
async fn connect_with_signer() -> Result<BreezSdk, SdkError> {
    // Create the signer
    let signer = default_external_signer(
        "<mnemonic words>".to_string(),
        Some("<optional passphrase>".to_string()),
        Network::Mainnet,
        Some(KeySetConfig {
            key_set_type: KeySetType::Default,
            use_address_index: false,
            account_number: None,
        }),
    )?;
    
    // Create the config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());
    
    // Connect using the external signer
    let sdk = connect_with_signer(ConnectWithSignerRequest {
        config,
        signer,
        storage_dir: "./.data".to_string(),
    })
    .await?;
    
    Ok(sdk)
}
// ANCHOR_END: connect-with-signer
