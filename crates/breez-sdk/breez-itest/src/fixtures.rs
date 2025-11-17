use anyhow::Result;
use breez_sdk_spark::{Fee, Network, default_config};
use rand::RngCore;
use rstest::fixture;
use tempdir::TempDir;
use tracing::info;

use crate::{SdkInstance, build_sdk_with_custom_config, build_sdk_with_dir};

/// Fixture: Alice's SDK with temporary storage
#[fixture]
pub async fn alice_sdk() -> Result<SdkInstance> {
    let alice_dir = TempDir::new("breez-sdk-alice")?;
    let path = alice_dir.path().to_string_lossy().to_string();

    // Generate random seed for Alice
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    info!("Initializing Alice's SDK at: {} with random seed", path);
    build_sdk_with_dir(path, seed, Some(alice_dir)).await
}

/// Fixture: Bob's SDK with temporary storage
#[fixture]
pub async fn bob_sdk() -> Result<SdkInstance> {
    let bob_dir = TempDir::new("breez-sdk-bob")?;
    let path = bob_dir.path().to_string_lossy().to_string();

    // Generate random seed for Bob
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    info!("Initializing Bob's SDK at: {} with random seed", path);
    build_sdk_with_dir(path, seed, Some(bob_dir)).await
}

#[fixture]
pub async fn bob_no_fee_sdk() -> Result<SdkInstance> {
    let dir = TempDir::new("breez-sdk-bob-no-fee")?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut cfg = default_config(Network::Regtest);
    cfg.max_deposit_claim_fee = None;
    build_sdk_with_custom_config(path, seed, cfg, Some(dir)).await
}

#[fixture]
pub async fn bob_strict_fee_sdk() -> Result<SdkInstance> {
    let dir = TempDir::new("breez-sdk-bob-fee")?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut cfg = default_config(Network::Regtest);
    cfg.max_deposit_claim_fee = Some(Fee::Fixed { amount: 0 });
    build_sdk_with_custom_config(path, seed, cfg, Some(dir)).await
}
