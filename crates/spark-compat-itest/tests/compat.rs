//! Backward-compatibility tests black-boxing the signer across SDK versions.
//!
//! `vold` is the previous release (git tag pinned in Cargo.toml), `vnew` the
//! current build. Each test starts a flow with one version, goes offline
//! (drops the wallet), and finishes the flow with the other version from the
//! same seed. A key-derivation or signing regression in either direction
//! makes the continuation fail.

use std::sync::Arc;

use anyhow::Result;
use spark_compat_itest::{FUND_SATS, TestFixtures, random_seed, vnew, vold};

/// Same seed must produce identical keys in both builds: identity key, leaf
/// signing keys (for fixed leaf ids), and static-deposit keys, on every
/// network. Catches derivation drift without any infrastructure.
#[test_log::test(tokio::test)]
async fn key_derivation_equivalence() -> Result<()> {
    use spark_wallet::SparkSigner as _;
    use spark_wallet_old::Signer as _;

    let networks = [
        (
            spark_wallet_old::Network::Regtest,
            spark_wallet::Network::Regtest,
        ),
        (
            spark_wallet_old::Network::Mainnet,
            spark_wallet::Network::Mainnet,
        ),
    ];

    for (old_network, new_network) in networks {
        for seed_byte in [1u8, 42, 255] {
            let seed = [seed_byte; 32];
            let old_signer = spark_wallet_old::DefaultSigner::new(&seed, old_network)?;
            let new_signer = spark_wallet::SparkSignerAdapter::new(Arc::new(
                spark_wallet::DefaultSigner::new(&seed, new_network)?,
            ));

            assert_eq!(
                old_signer.get_identity_public_key().await?,
                new_signer.get_identity_public_key().await?,
                "identity key diverged (network {new_network:?}, seed byte {seed_byte})",
            );

            for leaf in [
                "a",
                "leaf-2",
                "deadbeef-cafe",
                "0193e9a3-1f37-7000-8000-000000000001",
            ] {
                let old_id: spark_wallet_old::TreeNodeId = leaf.parse().expect("old leaf id");
                let new_id: spark_wallet::TreeNodeId = leaf.parse().expect("new leaf id");
                assert_eq!(
                    old_signer.get_public_key_for_node(&old_id).await?,
                    new_signer.get_public_key_for_leaf(&new_id).await?,
                    "leaf signing key diverged for leaf id {leaf:?} (network {new_network:?}, seed byte {seed_byte})",
                );
            }

            for index in [0u32, 1, 2, 7, 100] {
                assert_eq!(
                    old_signer.static_deposit_signing_key(index).await?,
                    new_signer.get_static_deposit_public_key(index).await?,
                    "static deposit key diverged at index {index} (network {new_network:?}, seed byte {seed_byte})",
                );
            }
        }
    }

    Ok(())
}

/// vold generates a deposit address, the funds arrive while offline, and vnew
/// (same seed) claims: the new build must re-derive the leaf key the old
/// build registered with the operators.
#[test_log::test(tokio::test)]
async fn deposit_initiated_old_claimed_new() -> Result<()> {
    let fx = TestFixtures::new().await?;
    let seed = random_seed();

    let (tx, vout) = {
        let wallet = vold::wallet(&fx, &seed).await?;
        vold::unclaimed_deposit(&wallet, &fx.bitcoind).await?
        // dropped: offline before the claim
    };

    let wallet = vnew::wallet(&fx, &seed).await?;
    vnew::claim_deposit(&wallet, &fx.bitcoind, tx, vout).await?;

    Ok(())
}

/// Reverse direction: vnew generates the deposit address, vold claims it.
/// State created by the new build must stay usable by the released build.
#[test_log::test(tokio::test)]
async fn deposit_initiated_new_claimed_old() -> Result<()> {
    let fx = TestFixtures::new().await?;
    let seed = random_seed();

    let (tx, vout) = {
        let wallet = vnew::wallet(&fx, &seed).await?;
        vnew::unclaimed_deposit(&wallet, &fx.bitcoind).await?
    };

    let wallet = vold::wallet(&fx, &seed).await?;
    vold::claim_deposit(&wallet, &fx.bitcoind, tx, vout).await?;

    Ok(())
}

/// vold creates a Spark invoice and goes offline; the invoice is paid
/// externally (an old-build payer); vnew (same seed) comes online and claims
/// the pending transfer: it must decrypt the leaf-key ciphertext with the
/// identity/ECIES key and derive the same new leaf keys.
#[test_log::test(tokio::test)]
async fn spark_invoice_created_old_claimed_new() -> Result<()> {
    let fx = TestFixtures::new().await?;
    let receiver_seed = random_seed();
    let payer_seed = random_seed();

    let payer = vold::wallet(&fx, &payer_seed).await?;
    vold::fund(&payer, &fx.bitcoind).await?;

    let invoice = {
        let receiver = vold::wallet(&fx, &receiver_seed).await?;
        vold::create_invoice(&receiver).await?
        // dropped: receiver offline while the invoice gets paid
    };

    vold::pay_invoice(&payer, &invoice).await?;

    let receiver = vnew::wallet(&fx, &receiver_seed).await?;
    vnew::await_balance(&receiver, FUND_SATS).await?;

    Ok(())
}

/// Reverse direction: vnew creates the invoice and a vnew payer pays it; vold
/// (same seed) claims. Invoices signed by the new build must verify and be
/// claimable on the released build.
#[test_log::test(tokio::test)]
async fn spark_invoice_created_new_claimed_old() -> Result<()> {
    let fx = TestFixtures::new().await?;
    let receiver_seed = random_seed();
    let payer_seed = random_seed();

    let payer = vnew::wallet(&fx, &payer_seed).await?;
    vnew::fund(&payer, &fx.bitcoind).await?;

    let invoice = {
        let receiver = vnew::wallet(&fx, &receiver_seed).await?;
        vnew::create_invoice(&receiver).await?
    };

    vnew::pay_invoice(&payer, &invoice).await?;

    let receiver = vold::wallet(&fx, &receiver_seed).await?;
    vold::await_balance(&receiver, FUND_SATS).await?;

    Ok(())
}

/// Full leaf-continuity round trip:
/// 1. vold A deposits and claims, so A's leaves were created by the old build.
/// 2. vnew A (same seed) discovers and spends those leaves to B (new sender).
/// 3. vold B (fresh seed) claims the transfer sent by the new build.
/// 4. vold B spends the leaves it just claimed back to A (old sender).
/// 5. vnew A claims the transfer sent by the old build.
#[test_log::test(tokio::test)]
async fn leaves_cross_version_spend_round_trip() -> Result<()> {
    let fx = TestFixtures::new().await?;
    let seed_a = random_seed();
    let seed_b = random_seed();

    {
        let a_old = vold::wallet(&fx, &seed_a).await?;
        vold::fund(&a_old, &fx.bitcoind).await?;
        // dropped: A's old-build leaves stay with the operators
    }

    let a_new = vnew::wallet(&fx, &seed_a).await?;
    vnew::await_balance(&a_new, FUND_SATS).await?;
    let a_address = vnew::address_string(&a_new)?;

    let b_old = vold::wallet(&fx, &seed_b).await?;
    let b_address = vold::address_string(&b_old)?;

    // New build spends the leaves the old build created.
    vnew::send_all(&a_new, &b_address).await?;
    vnew::await_balance(&a_new, 0).await?;

    // Old build claims a transfer initiated by the new build, then spends the
    // claimed leaves straight back.
    vold::await_balance(&b_old, FUND_SATS).await?;
    vold::send_all(&b_old, &a_address).await?;
    vold::await_balance(&b_old, 0).await?;

    // New build claims a transfer initiated by the old build.
    vnew::await_balance(&a_new, FUND_SATS).await?;

    Ok(())
}
