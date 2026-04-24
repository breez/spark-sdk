//! Full integration tests for `BreezSdk::prepare_unilateral_exit` against a
//! local Spark operator pool (via the spark-itest fixture) and a regtest
//! bitcoind. Each scenario spins up fresh fixtures, builds a `BreezSdk`
//! pointed at them, claims one or two leaves, and exercises the exit flow
//! with a specific on-chain confirmation structure.

use std::sync::Arc;

use anyhow::Result;
use bitcoin::{Amount, Transaction, Txid, consensus::encode::deserialize};
use breez_sdk_itest::{LocalSdk, build_local_sdk};
use breez_sdk_spark::{
    PrepareUnilateralExitRequest, PrepareUnilateralExitResponse, UnilateralExitCpfpInput,
};
use rstest::*;
use spark_itest::fixtures::setup::TestFixtures;
use spark_itest::helpers::{
    FundedUtxo, P2TR_INPUT_WEIGHT, deposit_with_amount, fund_p2tr_utxo, make_cpfp_input,
    submit_package_with_csv_retry,
};

const FEE_RATE: u64 = 1;
const LEAF_SATS: u64 = 200_000;
const CPFP_SATS: u64 = 200_000;

async fn new_local_sdk() -> Result<LocalSdk> {
    let fixtures = Arc::new(TestFixtures::new().await?);
    let mut seed = [0u8; 32];
    rand::Rng::fill(&mut rand::thread_rng(), &mut seed[..]);
    build_local_sdk(fixtures, seed).await
}

/// Fund a regtest deposit address then claim it to populate a single leaf
/// into the user's Spark tree. Goes through the underlying `SparkWallet`
/// because the local fixture's SSP stub has no URL wired up; the BreezSdk's
/// public `claim_deposit` would try to fetch a fee quote and fail.
async fn deposit_and_claim(sdk: &LocalSdk, amount: Amount) -> Result<()> {
    deposit_with_amount(&sdk.spark_wallet, &sdk.fixtures.bitcoind, amount.to_sat()).await
}

fn cpfp_input(utxo: &FundedUtxo) -> UnilateralExitCpfpInput {
    UnilateralExitCpfpInput::P2tr {
        txid: utxo.outpoint.txid.to_string(),
        vout: utxo.outpoint.vout,
        value: utxo.witness_utxo.value.to_sat(),
        pubkey: hex::encode(
            utxo.secret_key
                .public_key(&bitcoin::key::Secp256k1::new())
                .serialize(),
        ),
    }
}

async fn prepare_exit(sdk: &LocalSdk, utxo: &FundedUtxo) -> Result<PrepareUnilateralExitResponse> {
    let signer =
        breez_sdk_spark::signer::single_key_cpfp_signer(utxo.secret_key.secret_bytes().to_vec())?;
    let resp = sdk
        .sdk
        .prepare_unilateral_exit(
            PrepareUnilateralExitRequest {
                fee_rate_sat_per_vbyte: FEE_RATE,
                inputs: vec![cpfp_input(utxo)],
                destination: utxo.address.to_string(),
            },
            signer,
        )
        .await?;
    Ok(resp)
}

fn decode_tx(hex_str: &str) -> Result<Transaction> {
    let bytes = hex::decode(hex_str)?;
    Ok(deserialize::<Transaction>(&bytes)?)
}

/// Broadcast a single parent+child transaction pair and mine 1 block to
/// confirm the package.
async fn broadcast_and_mine(
    sdk: &LocalSdk,
    parent: &Transaction,
    child: &Transaction,
) -> Result<()> {
    submit_package_with_csv_retry(&sdk.fixtures.bitcoind, parent, child).await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;
    Ok(())
}

fn txid_of(hex_str: &str) -> Result<Txid> {
    Ok(decode_tx(hex_str)?.compute_txid())
}

/// Scenario A — no prior broadcast. Both node_tx and refund_tx entries must
/// be present in the response with their CPFP children.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_nothing_confirmed() -> Result<()> {
    let sdk = new_local_sdk().await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let resp = prepare_exit(&sdk, &cpfp_utxo).await?;
    assert_eq!(resp.leaves.len(), 1, "expected a single selected leaf");
    let leaf = &resp.leaves[0];
    assert_eq!(
        leaf.transactions.len(),
        2,
        "expected node_tx + refund_tx entries, got {}",
        leaf.transactions.len()
    );
    for tx in &leaf.transactions {
        assert!(
            tx.cpfp_tx_hex.is_some(),
            "all entries must have a CPFP child when nothing is on-chain"
        );
    }
    let refund_entry = leaf
        .transactions
        .last()
        .expect("at least one transaction entry");
    assert!(
        refund_entry.csv_timelock_blocks.is_some() && refund_entry.csv_timelock_blocks.unwrap() > 0,
        "refund entry must carry a positive CSV timelock"
    );
    assert!(!resp.sweep_tx_hex.is_empty());
    Ok(())
}

/// Scenario B — broadcast + confirm only the node_tx package. The second
/// `prepare_unilateral_exit` call must keep the node_tx entry (now with
/// `cpfp_tx_hex: None`, per the docs' skip contract) and still return a CPFP
/// for the unconfirmed refund_tx.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_only_node_tx_confirmed() -> Result<()> {
    let sdk = new_local_sdk().await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let first = prepare_exit(&sdk, &cpfp_utxo).await?;
    let node_tx_entry = &first.leaves[0].transactions[0];
    let node_tx = decode_tx(&node_tx_entry.tx_hex)?;
    let node_child = decode_tx(
        node_tx_entry
            .cpfp_tx_hex
            .as_ref()
            .expect("CPFP child must be set in the first pass"),
    )?;
    broadcast_and_mine(&sdk, &node_tx, &node_child).await?;

    // Build a fresh CPFP input because the first pass's PSBT chain spent ours.
    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let second = prepare_exit(&sdk, &cpfp_utxo).await?;
    assert_eq!(second.leaves.len(), 1);
    let leaf = &second.leaves[0];

    let node_txid = node_tx.compute_txid();
    let refund_txid = txid_of(&first.leaves[0].transactions[1].tx_hex)?;
    assert_eq!(
        leaf.transactions.len(),
        2,
        "confirmed node_tx must still appear (with cpfp_tx_hex = None); refund remains; got {} entries",
        leaf.transactions.len()
    );
    let node_entry = &leaf.transactions[0];
    let refund_entry = &leaf.transactions[1];
    assert_eq!(txid_of(&node_entry.tx_hex)?, node_txid);
    assert!(
        node_entry.cpfp_tx_hex.is_none(),
        "confirmed node_tx entry must carry cpfp_tx_hex = None"
    );
    assert_eq!(txid_of(&refund_entry.tx_hex)?, refund_txid);
    assert!(
        refund_entry.cpfp_tx_hex.is_some(),
        "unconfirmed refund entry must carry its CPFP child"
    );
    Ok(())
}

/// Scenario C — broadcast + confirm both node_tx and refund_tx, then broadcast
/// the sweep from the same response. Exercises the end-to-end flow where every
/// package plus the final sweep lands on-chain. After the refund confirms, the
/// Spark tree service marks the leaf as on-chain and it no longer appears in
/// `list_leaves().available`; the integrator sweeps using the `sweep_tx_hex`
/// returned by the initial `prepare_unilateral_exit` call.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_both_confirmed() -> Result<()> {
    let sdk = new_local_sdk().await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let prepared = prepare_exit(&sdk, &cpfp_utxo).await?;
    for entry in &prepared.leaves[0].transactions {
        let parent = decode_tx(&entry.tx_hex)?;
        let child = decode_tx(
            entry
                .cpfp_tx_hex
                .as_ref()
                .expect("CPFP child must be set in the first pass"),
        )?;
        broadcast_and_mine(&sdk, &parent, &child).await?;
    }

    let sweep_tx = decode_tx(&prepared.sweep_tx_hex)?;
    let sweep_txid = sdk
        .fixtures
        .bitcoind
        .broadcast_transaction(&sweep_tx)
        .await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;
    let confirmed = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    assert_eq!(confirmed.compute_txid(), sweep_txid);
    Ok(())
}

/// Scenario D — two independent deposits. Broadcast only leaf A's node_tx.
/// The response must list both leaves; leaf A loses its node_tx entry while
/// leaf B keeps both. Confirms multi-leaf handling of mixed on-chain states.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_multi_leaf_mixed() -> Result<()> {
    let sdk = new_local_sdk().await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS * 2)).await?;

    let first = prepare_exit(&sdk, &cpfp_utxo).await?;
    assert_eq!(first.leaves.len(), 2, "expected two selected leaves");

    let target_leaf_id = first.leaves[0].leaf_id.clone();
    let node_entry = &first.leaves[0].transactions[0];
    let node_tx = decode_tx(&node_entry.tx_hex)?;
    let node_child = decode_tx(
        node_entry
            .cpfp_tx_hex
            .as_ref()
            .expect("CPFP child must be set in the first pass"),
    )?;
    broadcast_and_mine(&sdk, &node_tx, &node_child).await?;

    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(200_000)).await?;
    let second = prepare_exit(&sdk, &cpfp_utxo).await?;
    assert_eq!(
        second.leaves.len(),
        2,
        "both leaves must still be selected after the partial broadcast"
    );

    let mut saw_partial = false;
    let mut saw_full = false;
    for leaf in &second.leaves {
        if leaf.leaf_id == target_leaf_id {
            assert_eq!(
                leaf.transactions.len(),
                2,
                "leaf A keeps both entries; node_tx carries cpfp_tx_hex = None"
            );
            assert!(
                leaf.transactions[0].cpfp_tx_hex.is_none(),
                "leaf A's confirmed node_tx entry must carry cpfp_tx_hex = None"
            );
            assert!(
                leaf.transactions[1].cpfp_tx_hex.is_some(),
                "leaf A's refund must still carry its CPFP child"
            );
            saw_partial = true;
        } else {
            assert_eq!(
                leaf.transactions.len(),
                2,
                "leaf B is entirely unconfirmed, should keep both entries"
            );
            assert!(
                leaf.transactions.iter().all(|t| t.cpfp_tx_hex.is_some()),
                "leaf B's entries must all carry their CPFP children"
            );
            saw_full = true;
        }
    }
    assert!(saw_partial && saw_full, "must observe both leaf states");
    Ok(())
}

/// Scenario E — the refund was satisfied by `direct_from_cpfp_refund_tx`
/// instead of the CPFP `refund_tx`. Verify `SparkWallet::unilateral_exit`
/// adapts the sweep to the direct-refund outpoint when that variant is in
/// `confirmed_spenders`. Exercises the wallet method directly: once any
/// refund lands on-chain the Spark operators flag the leaf as `OnChain` and
/// autoselect no longer picks it, so routing this through
/// `BreezSdk::prepare_unilateral_exit` isn't meaningful.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_sweep_adapts_to_direct_from_cpfp_refund() -> Result<()> {
    let sdk = new_local_sdk().await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp_utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    // Pull the optimistic exit path so we can reach into the refund entry's
    // hydrated `known_spenders` and pick out `direct_from_cpfp_refund_tx`.
    let optimistic = sdk
        .spark_wallet
        .unilateral_exit_autoselect(
            FEE_RATE,
            vec![make_cpfp_input(&cpfp_utxo, P2TR_INPUT_WEIGHT)],
            cpfp_utxo.address.clone(),
        )
        .await?;
    assert_eq!(optimistic.leaf_tx_cpfp_psbts.len(), 1);
    let leaf_psbts = &optimistic.leaf_tx_cpfp_psbts[0];
    assert_eq!(leaf_psbts.tx_cpfp_psbts.len(), 2);

    let node_entry = &leaf_psbts.tx_cpfp_psbts[0];
    let refund_entry = &leaf_psbts.tx_cpfp_psbts[1];
    let node_tx = node_entry.parent_tx.clone();
    let cpfp_refund_tx = refund_entry.parent_tx.clone();

    // direct_from_cpfp_refund_tx spends the same outpoint as refund_tx but
    // with a higher-sequence CSV. Pick it out of known_spenders.
    let cpfp_refund_input = cpfp_refund_tx.input[0].previous_output;
    let direct_from_cpfp_refund_tx: Transaction = refund_entry
        .known_spenders
        .iter()
        .find(|tx| {
            tx.input
                .iter()
                .any(|i| i.previous_output == cpfp_refund_input)
        })
        .expect("direct_from_cpfp_refund_tx must be present in known_spenders")
        .clone();

    let selected_ids: Vec<_> = optimistic
        .selected_leaves
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let result = sdk
        .spark_wallet
        .unilateral_exit(
            FEE_RATE,
            selected_ids,
            vec![make_cpfp_input(&cpfp_utxo, P2TR_INPUT_WEIGHT)],
            cpfp_utxo.address.clone(),
            Some(optimistic.prefetched_nodes.clone()),
            vec![node_tx.clone(), direct_from_cpfp_refund_tx.clone()],
        )
        .await?;

    assert_eq!(result.leaves.len(), 1);
    let leaf = &result.leaves[0];
    assert_eq!(leaf.tx_cpfp_psbts.len(), 2);
    for entry in &leaf.tx_cpfp_psbts {
        assert!(
            entry.child_psbt.is_none(),
            "consumed step must carry child_psbt = None"
        );
    }

    let direct_refund_txid = direct_from_cpfp_refund_tx.compute_txid();
    assert!(
        result
            .sweep_tx
            .input
            .iter()
            .any(|i| i.previous_output.txid == direct_refund_txid && i.previous_output.vout == 0),
        "sweep must spend direct_from_cpfp_refund_tx's output[0]; inputs: {:?}",
        result.sweep_tx.input
    );
    Ok(())
}
