use anyhow::Result;
use breez_sdk_spark::turnkey::{create_turnkey_signer, TurnkeyConfig};
use breez_sdk_spark::*;

async fn connect_with_turnkey() -> Result<BreezSdk> {
    // ANCHOR: turnkey-connect
    let turnkey_config = TurnkeyConfig {
        base_url: None,
        organization_id: "<turnkey sub-organization id>".to_string(),
        api_public_key: "<api public key hex>".to_string(),
        api_private_key: "<api private key hex>".to_string(),
        wallet_id: "<turnkey wallet id>".to_string(),
        network: Network::Mainnet,
        account_number: None,
        // Set after the first connect to make later signer setup network-free
        identity_public_key: None,
        retry: None,
        max_rps: None,
    };

    let signers = create_turnkey_signer(turnkey_config).await?;

    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    let sdk = connect_with_signer(ConnectWithSignerRequest {
        config,
        breez_signer: signers.breez_signer,
        spark_signer: signers.spark_signer,
        storage_dir: "./.data".to_string(),
    })
    .await?;
    // ANCHOR_END: turnkey-connect
    Ok(sdk)
}
