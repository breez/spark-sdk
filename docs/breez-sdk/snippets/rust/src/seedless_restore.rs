use anyhow::Result;
use breez_sdk_spark::seedless_restore::{PasskeyPrfError, PasskeyPrfProvider, SeedlessRestore};
use breez_sdk_spark::{default_config, Network, SdkBuilder, Seed};
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

async fn create_seed() -> Result<Seed> {
    // ANCHOR: create-seed
    let prf_provider = Arc::new(ExamplePasskeyPrfProvider);
    let seedless = SeedlessRestore::new(prf_provider, None);

    // Create a new seed with user-chosen salt
    // The salt is published to Nostr for later discovery
    let seed = seedless.create_seed("personal".to_string()).await?;

    // Use the seed to initialize the SDK
    let config = default_config(Network::Mainnet);
    let _sdk = SdkBuilder::new(config, seed.clone())
        .with_default_storage("./.data".to_string())
        .build()
        .await?;
    // ANCHOR_END: create-seed
    Ok(seed)
}

async fn list_salts() -> Result<Vec<String>> {
    // ANCHOR: list-salts
    let prf_provider = Arc::new(ExamplePasskeyPrfProvider);
    let seedless = SeedlessRestore::new(prf_provider, None);

    // Query Nostr for salts associated with this passkey
    let salts = seedless.list_salts().await?;

    for salt in &salts {
        println!("Found wallet: {}", salt);
    }
    // ANCHOR_END: list-salts
    Ok(salts)
}

async fn restore_seed() -> Result<Seed> {
    // ANCHOR: restore-seed
    let prf_provider = Arc::new(ExamplePasskeyPrfProvider);
    let seedless = SeedlessRestore::new(prf_provider, None);

    // Restore seed using a known salt
    let seed = seedless.restore_seed("personal".to_string()).await?;

    // Use the seed to initialize the SDK
    let config = default_config(Network::Mainnet);
    let _sdk = SdkBuilder::new(config, seed.clone())
        .with_default_storage("./.data".to_string())
        .build()
        .await?;
    // ANCHOR_END: restore-seed
    Ok(seed)
}
