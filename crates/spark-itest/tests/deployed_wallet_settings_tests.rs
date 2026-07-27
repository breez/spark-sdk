use anyhow::Result;
use bitcoin::Address;
use rstest::*;
use spark_itest::{
    faucet::RegtestFaucet,
    helpers::{create_regtest_wallet, wait_for_event},
    mempool::MempoolClient,
};
use spark_wallet::{MasterIdentityPublicKeyUpdate, WalletEvent};
use tracing::info;

const DEPOSIT_AMOUNT_SATS: u64 = 50_000;

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

    let (owner, mut owner_listener) = create_regtest_wallet().await?;
    let (reader, _reader_listener) = create_regtest_wallet().await?;

    let owner_identity = owner.get_identity_public_key();
    let reader_identity = reader.get_identity_public_key();

    // Fund the owner, so there is a balance worth hiding.
    let deposit = owner.generate_deposit_address().await?;
    let deposit_address = &deposit.address;
    let txid = faucet
        .fund_address(&deposit_address.to_string(), DEPOSIT_AMOUNT_SATS)
        .await?;
    let tx = mempool.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| {
            Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
                .is_ok_and(|addr| &addr == deposit_address)
        })
        .map(|(i, _)| i as u32)
        .expect("Could not find deposit address in transaction outputs");
    owner.claim_deposit(tx, vout).await?;
    wait_for_event(&mut owner_listener, 180, "DepositConfirmed", |e| match e {
        WalletEvent::DepositConfirmed(_) => Ok(Some(e)),
        _ => Ok(None),
    })
    .await?;
    assert_eq!(owner.get_balance().await?, DEPOSIT_AMOUNT_SATS);

    owner.update_wallet_settings(Some(true), None).await?;
    let settings = owner.query_wallet_settings().await?;
    assert!(settings.private_enabled);
    assert_eq!(settings.master_identity_public_key, None);

    // The owner always reads its own wallet, private mode or not.
    assert_eq!(
        owner.query_available_balance_of(&owner_identity).await?,
        DEPOSIT_AMOUNT_SATS,
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
        DEPOSIT_AMOUNT_SATS,
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
