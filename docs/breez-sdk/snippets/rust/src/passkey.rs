use anyhow::Result;
use breez_sdk_spark::passkey::{
    NostrRelayConfig, PasskeyPrfError, PasskeyPrfProvider, Passkey,
};
use breez_sdk_spark::{connect, default_config, ConnectRequest, Network};
use std::sync::Arc;

// ANCHOR: implement-prf-provider
/// In practice, implement using platform-specific passkey APIs.
struct ExamplePasskeyPrfProvider;

#[async_trait::async_trait]
impl PasskeyPrfProvider for ExamplePasskeyPrfProvider {
    async fn derive_prf_seed(&self, _salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        // Call platform passkey API with PRF extension
        // Returns 32-byte PRF output
        todo!("Implement using WebAuthn or native passkey APIs")
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        // Check if PRF-capable passkey exists
        todo!("Check platform passkey availability")
    }
}
// ANCHOR_END: implement-prf-provider

async fn connect_with_passkey() -> Result<breez_sdk_spark::BreezSdk> {
    // ANCHOR: connect-with-passkey
    let prf_provider = Arc::new(ExamplePasskeyPrfProvider);
    let passkey = Passkey::new(prf_provider, None);

    // Derive the wallet from the passkey (pass None for the default wallet)
    let wallet = passkey.get_wallet(Some("personal".to_string())).await?;

    let config = default_config(Network::Mainnet);
    let sdk = connect(ConnectRequest {
        config,
        seed: wallet.seed,
        storage_dir: "./.data".to_string(),
    })
    .await?;
    // ANCHOR_END: connect-with-passkey
    Ok(sdk)
}

async fn list_wallet_names() -> Result<Vec<String>> {
    // ANCHOR: list-wallet-names
    let prf_provider = Arc::new(ExamplePasskeyPrfProvider);
    let relay_config = NostrRelayConfig {
        breez_api_key: Some("<breez api key>".to_string()),
        ..NostrRelayConfig::default()
    };
    let passkey = Passkey::new(prf_provider, Some(relay_config));

    // Query Nostr for wallet names associated with this passkey
    let wallet_names = passkey.list_wallet_names().await?;

    for wallet_name in &wallet_names {
        println!("Found wallet: {}", wallet_name);
    }
    // ANCHOR_END: list-wallet-names
    Ok(wallet_names)
}

async fn store_wallet_name() -> Result<()> {
    // ANCHOR: store-wallet-name
    let prf_provider = Arc::new(ExamplePasskeyPrfProvider);
    let relay_config = NostrRelayConfig {
        breez_api_key: Some("<breez api key>".to_string()),
        ..NostrRelayConfig::default()
    };
    let passkey = Passkey::new(prf_provider, Some(relay_config));

    // Publish the wallet name to Nostr for later discovery
    passkey.store_wallet_name("personal".to_string()).await?;
    // ANCHOR_END: store-wallet-name
    Ok(())
}
