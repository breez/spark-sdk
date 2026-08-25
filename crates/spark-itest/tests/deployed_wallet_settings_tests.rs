use anyhow::Result;
use rstest::*;
use spark_itest::{
    faucet::RegtestFaucet,
    helpers::{create_regtest_wallet, fund_wallet_via_static_deposit, wait_for},
    mempool::MempoolClient,
};
use spark_wallet::MasterIdentityPublicKeyUpdate;
use tracing::info;

const DEPOSIT_AMOUNT_SATS: u64 = 50_000;
/// Generous, since it covers the SSP picking up the funding transaction. The
/// waits it bounds poll, so a healthy run returns well inside it.
const DEPOSIT_TIMEOUT_SECS: u64 = 300;

/// Exercises the operator-side read access granted by a wallet's master
/// identity.
///
/// `owner` funds itself and enables private mode, which hides its balance from
/// every session but its own, then designates `reader` as its master identity.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_master_identity_grants_read_access() -> Result<()> {
    info!("=== Starting test_master_identity_grants_read_access ===");

    let faucet = RegtestFaucet::new()?;
    let mempool = MempoolClient::new()?;

    let (owner, _owner_listener) = create_regtest_wallet().await?;
    let (reader, _reader_listener) = create_regtest_wallet().await?;

    let owner_identity = owner.get_identity_public_key();
    let reader_identity = reader.get_identity_public_key();

    // Fund the owner, so there is a balance worth hiding.
    let credited_sats = fund_wallet_via_static_deposit(
        &owner,
        &faucet,
        &mempool,
        DEPOSIT_AMOUNT_SATS,
        DEPOSIT_TIMEOUT_SECS,
    )
    .await?;
    wait_for(
        || async { owner.get_balance().await.is_ok_and(|b| b == credited_sats) },
        DEPOSIT_TIMEOUT_SECS,
        "the claimed static deposit to land in the owner's balance",
    )
    .await?;

    owner.update_wallet_settings(Some(true), None).await?;
    let settings = owner.query_wallet_settings().await?;
    assert!(settings.private_enabled);
    assert_eq!(settings.master_identity_public_key, None);

    // The owner always reads its own wallet, private mode or not.
    assert_eq!(
        owner.query_available_balance_of(&owner_identity).await?,
        credited_sats,
        "the owner must see its own balance under private mode"
    );

    // No master identity designated yet, so the reader is blind.
    assert_eq!(
        reader.query_available_balance_of(&owner_identity).await?,
        0,
        "a private wallet must not be readable before a master identity is designated"
    );

    owner
        .update_wallet_settings(
            None,
            Some(MasterIdentityPublicKeyUpdate::Set(reader_identity)),
        )
        .await?;

    // Designating a master identity must not disturb private mode.
    let settings = owner.query_wallet_settings().await?;
    assert!(settings.private_enabled);
    assert_eq!(settings.master_identity_public_key, Some(reader_identity));

    assert_eq!(
        reader.query_available_balance_of(&owner_identity).await?,
        credited_sats,
        "the designated master identity must read the private wallet's balance"
    );

    owner
        .update_wallet_settings(None, Some(MasterIdentityPublicKeyUpdate::Clear))
        .await?;

    let settings = owner.query_wallet_settings().await?;
    assert!(settings.private_enabled);
    assert_eq!(settings.master_identity_public_key, None);

    assert_eq!(
        reader.query_available_balance_of(&owner_identity).await?,
        0,
        "clearing the master identity must revoke read access"
    );

    info!("=== Test test_master_identity_grants_read_access PASSED ===");
    Ok(())
}
