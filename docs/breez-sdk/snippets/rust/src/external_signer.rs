use std::sync::Arc;
use breez_sdk_spark::signer::ExternalSigner;
use breez_sdk_spark::*;

// ANCHOR: default-external-signer
fn create_signer() -> Result<Arc<dyn ExternalSigner>, SdkError> {
    let mnemonic = "<mnemonic words>".to_string();
    let network = Network::Mainnet;
    
    let signer = default_external_signer(
        mnemonic,
        None, // passphrase
        network,
        Some(KeySetConfig {
            key_set_type: KeySetType::Default,
            use_address_index: false,
            account_number: Some(0),
        }),
    )?;
    
    Ok(signer)
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
async fn connect_example() -> Result<BreezSdk, SdkError> {
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
