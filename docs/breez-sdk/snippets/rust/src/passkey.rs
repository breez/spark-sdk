use anyhow::Result;
use breez_sdk_spark::passkey::{
    NostrRelayConfig, PasskeyPrfError, PasskeyPrfProvider, Passkey,
};
use breez_sdk_spark::{connect, default_config, ConnectRequest, Network};
use std::sync::Arc;

// ANCHOR: implement-prf-provider
/// Implement using platform-specific passkey APIs if the SDK does not ship a built-in provider for your target.
struct CustomPasskeyPrfProvider;

#[async_trait::async_trait]
impl PasskeyPrfProvider for CustomPasskeyPrfProvider {
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

async fn check_availability() -> Result<()> {
    // ANCHOR: check-availability
    let prf_provider = Arc::new(CustomPasskeyPrfProvider);

    if prf_provider.is_prf_available().await? {
        // Show passkey as primary option
    } else {
        // Fall back to mnemonic flow
    }
    // ANCHOR_END: check-availability
    Ok(())
}

async fn connect_with_passkey() -> Result<breez_sdk_spark::BreezSdk> {
    // ANCHOR: connect-with-passkey
    let prf_provider = Arc::new(CustomPasskeyPrfProvider);
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

async fn list_labels() -> Result<Vec<String>> {
    // ANCHOR: list-labels
    let prf_provider = Arc::new(CustomPasskeyPrfProvider);
    let relay_config = NostrRelayConfig {
        breez_api_key: Some("<breez api key>".to_string()),
        ..NostrRelayConfig::default()
    };
    let passkey = Passkey::new(prf_provider, Some(relay_config));

    // Query Nostr for labels associated with this passkey
    let labels = passkey.list_labels().await?;

    for label in &labels {
        println!("Found label: {}", label);
    }
    // ANCHOR_END: list-labels
    Ok(labels)
}

async fn store_label() -> Result<()> {
    // ANCHOR: store-label
    let prf_provider = Arc::new(CustomPasskeyPrfProvider);
    let relay_config = NostrRelayConfig {
        breez_api_key: Some("<breez api key>".to_string()),
        ..NostrRelayConfig::default()
    };
    let passkey = Passkey::new(prf_provider, Some(relay_config));

    // Publish the label to Nostr for later discovery
    passkey.store_label("personal".to_string()).await?;
    // ANCHOR_END: store-label
    Ok(())
}
