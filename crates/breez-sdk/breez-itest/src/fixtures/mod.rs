pub mod data_sync;
pub mod docker;
pub mod lnurl;

use anyhow::Result;
use breez_sdk_spark::{MaxFee, Network, StableBalanceConfig, StableBalanceToken, default_config};
use rand::RngCore;
use rstest::fixture;
use tempdir::TempDir;
use tracing::info;

use crate::{
    SdkInstance, build_sdk_with_custom_config, build_sdk_with_dir, build_sdk_with_external_signer,
};

/// Token identifiers for regtest
pub const BEAN_REGTEST_TOKEN_ID: &str =
    "btknrt1muwlm2aeur2jhe4pkuh7v08jjaleqgeu69c5sk7p7qfhkywg7nkqerzl06";
pub const SHELL_REGTEST_TOKEN_ID: &str =
    "btknrt1ra8lrwpqgqfz7gcy3gfcucaw3fh62tp3d6qkjxafx0cnxm5gmd3q0xy27c";

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
    build_sdk_with_custom_config(path, seed, cfg, Some(dir), true).await
}

#[fixture]
pub async fn bob_strict_fee_sdk() -> Result<SdkInstance> {
    let dir = TempDir::new("breez-sdk-bob-fee")?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut cfg = default_config(Network::Regtest);
    cfg.max_deposit_claim_fee = Some(MaxFee::Fixed { amount: 0 });
    build_sdk_with_custom_config(path, seed, cfg, Some(dir), true).await
}

/// Fixture: Alice's SDK with external signer
#[fixture]
pub async fn alice_external_signer_sdk() -> Result<SdkInstance> {
    let alice_dir = TempDir::new("breez-sdk-alice-ext-signer")?;
    let path = alice_dir.path().to_string_lossy().to_string();

    let mnemonic = random_mnemonic()?;

    info!("Initializing Alice's SDK with external signer at: {}", path);
    build_sdk_with_external_signer(path, mnemonic, Some(alice_dir)).await
}

/// Fixture: Bob's SDK with external signer
#[fixture]
pub async fn bob_external_signer_sdk() -> Result<SdkInstance> {
    let bob_dir = TempDir::new("breez-sdk-bob-ext-signer")?;
    let path = bob_dir.path().to_string_lossy().to_string();

    let mnemonic = random_mnemonic()?;

    info!("Initializing Bob's SDK with external signer at: {}", path);
    build_sdk_with_external_signer(path, mnemonic, Some(bob_dir)).await
}

fn random_mnemonic() -> Result<String> {
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    Ok(bip39::Mnemonic::from_entropy(&entropy)?.to_string())
}

/// Fixture: Alice's SDK with stable balance config
#[fixture]
pub async fn alice_sdk_stable_balance() -> Result<SdkInstance> {
    let alice_dir = TempDir::new("breez-sdk-alice-stable-balance")?;
    let path = alice_dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut cfg = default_config(Network::Regtest);
    cfg.stable_balance_config = Some(StableBalanceConfig {
        tokens: vec![
            StableBalanceToken {
                label: "SHELL".to_string(),
                token_identifier: SHELL_REGTEST_TOKEN_ID.to_string(),
            },
            StableBalanceToken {
                label: "BEAN".to_string(),
                token_identifier: BEAN_REGTEST_TOKEN_ID.to_string(),
            },
        ],
        default_active_label: Some("SHELL".to_string()),
        threshold_sats: Some(1000),
        max_slippage_bps: Some(500),
    });
    build_sdk_with_custom_config(path, seed, cfg, Some(alice_dir), true).await
}
