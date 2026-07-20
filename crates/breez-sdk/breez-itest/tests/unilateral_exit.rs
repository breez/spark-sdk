//! Integration tests for the two-phase unilateral exit
//! (`BreezSdk::prepare_unilateral_exit` + `BreezSdk::unilateral_exit`) against a
//! local Spark operator pool and a regtest bitcoind.
//!
//! Every test drives the public two-phase API directly: it calls
//! `prepare_unilateral_exit` to obtain a quote, asserts the quote is internally
//! consistent (and, where a build follows, that the build matches it), then
//! calls `unilateral_exit` to produce the signed transaction set. The
//! end-to-end tests additionally broadcast that set to bitcoind and mine it, so
//! the transactions are proven minable, not merely well-formed.
//!
//! Gated behind `local-itest`: each test stands up its own operator cluster, so
//! it runs only under the low-concurrency `make itest`, never the 8-thread
//! `make breez-itest`.
#![cfg(feature = "local-itest")]

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use bitcoin::secp256k1::SecretKey;
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, OutPoint, Psbt, Sequence, Transaction, TxIn, TxOut, Txid, absolute::LockTime,
    consensus::encode::deserialize,
};
use breez_sdk_itest::{LocalSdk, SignerBackend, build_local_sdk, wait_for_balance};
use breez_sdk_spark::signer::{CpfpSigner, single_key_cpfp_signer};
use breez_sdk_spark::{
    ConfirmationStatus, CpfpFundingKind, CpfpInput, ExitLeafSelection,
    PrepareUnilateralExitRequest, PrepareUnilateralExitResponse, SdkError, UnilateralExitRequest,
    UnilateralExitResponse, UnilateralExitTransaction, UnilateralExitTxKind,
};
use rstest::*;
use rstest_reuse::{apply, template};
use spark_itest::fixtures::setup::TestFixtures;
use spark_itest::helpers::{
    FundedUtxo, deposit_with_amount, fund_p2tr_utxo, fund_p2tr_utxo_unmined,
    fund_p2tr_utxo_with_key, fund_p2wpkh_utxo, fund_p2wpkh_utxo_with_key, sign_cpfp_psbt_p2tr,
    submit_package_with_csv_retry,
};
use spark_wallet::{RefundOutput, is_ephemeral_anchor_output};

// The exit fee rate the tests build at, in sat/vByte (the public API unit). 1
// sat/vByte is the mainnet min-relay floor.
const FEE_RATE: u64 = 1;
// The same rate in sat/kW (250 sat/kW == 1 sat/vByte): the unit assert_fee_rate's
// weight-based math and direct spark-wallet calls (which take sat/kW) expect.
const FEE_RATE_KW: u64 = FEE_RATE * 250;
const LEAF_SATS: u64 = 200_000;
const CPFP_SATS: u64 = 200_000;

// The Turnkey case needs the `turnkey` feature and TURNKEY_* credentials;
// default builds run seed-only.
#[cfg(feature = "turnkey")]
#[template]
#[rstest]
#[case::seed(SignerBackend::Seed)]
#[case::turnkey(SignerBackend::Turnkey)]
fn each_backend(#[case] backend: SignerBackend) {}

#[cfg(not(feature = "turnkey"))]
#[template]
#[rstest]
#[case::seed(SignerBackend::Seed)]
fn each_backend(#[case] backend: SignerBackend) {}

async fn new_local_sdk(backend: SignerBackend) -> Result<LocalSdk> {
    let fixtures = Arc::new(TestFixtures::new().await?);
    build_local_sdk(fixtures, backend).await
}

/// Claims through the side-channel `SparkWallet`: the fixture's SSP stub has no
/// URL, so the public claim path (which fetches a fee quote) can't be used.
async fn deposit_and_claim(sdk: &LocalSdk, amount: Amount) -> Result<()> {
    deposit_with_amount(&sdk.spark_wallet, &sdk.fixtures.bitcoind, amount.to_sat()).await
}

/// The default single-key CPFP signer over `key`, as every test funds its exit.
/// A mechanical wrapper over the public `single_key_cpfp_signer`; it does not
/// stand in for the exit API itself, which every test calls directly.
fn signer_for(key: &[u8]) -> Result<Arc<dyn CpfpSigner>> {
    single_key_cpfp_signer(key.to_vec()).map_err(|e| anyhow::anyhow!("build signer: {e}"))
}

fn cpfp_input(utxo: &FundedUtxo) -> CpfpInput {
    CpfpInput::P2tr {
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

/// A `CpfpInput` matching a funded UTXO's script type (P2TR or P2WPKH).
fn cpfp_input_for(utxo: &FundedUtxo) -> CpfpInput {
    let pubkey = hex::encode(
        utxo.secret_key
            .public_key(&bitcoin::key::Secp256k1::new())
            .serialize(),
    );
    let txid = utxo.outpoint.txid.to_string();
    let vout = utxo.outpoint.vout;
    let value = utxo.witness_utxo.value.to_sat();
    if utxo.witness_utxo.script_pubkey.is_p2tr() {
        CpfpInput::P2tr {
            txid,
            vout,
            value,
            pubkey,
        }
    } else {
        CpfpInput::P2wpkh {
            txid,
            vout,
            value,
            pubkey,
        }
    }
}

/// A deterministic secret key, so several UTXOs can share one signer.
fn fixed_key(byte: u8) -> SecretKey {
    SecretKey::from_slice(&[byte; 32]).expect("valid secret key")
}

/// The P2TR (key-path) regtest address for `key`, used as an exit destination
/// independent of the funding UTXOs.
fn p2tr_address(key: &SecretKey) -> Address {
    let secp = bitcoin::key::Secp256k1::new();
    let (xonly, _) = key.public_key(&secp).x_only_public_key();
    Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest)
}

/// The dust limit of a P2TR output, the per-branch terminal-change reserve the
/// quote bakes into its funding figures for P2TR funding.
fn p2tr_dust() -> u64 {
    p2tr_address(&fixed_key(0x01))
        .script_pubkey()
        .minimal_non_dust()
        .to_sat()
}

fn decode_tx(hex_str: &str) -> Result<Transaction> {
    let bytes = hex::decode(hex_str)?;
    Ok(deserialize::<Transaction>(&bytes)?)
}

fn is_package(entry: &UnilateralExitTransaction) -> bool {
    matches!(
        entry.kind,
        UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund
    )
}

/// Internal consistency of a non-empty exit quote, independent of how it will be
/// funded: the summary fields agree with each other and echo the request.
/// `dust` is the funding kind's terminal-change dust reserve (330 for P2TR).
///
/// Grounded in `spark::services::quote_unilateral_exit`: `per_branch[i] = estimated_cost +
/// dust`, `single_utxo_funding = sum(per_branch) + fanout_fee`, `total_fee =
/// sum(estimated_cost) + fanout_fee`, so `single_utxo_funding - total_fee ==
/// n*dust` and `fanout_fee == 0` iff a single branch.
fn assert_quote_consistent(
    quote: &PrepareUnilateralExitResponse,
    fee_rate_sat_per_vbyte: u64,
    destination: &str,
    dust: u64,
) {
    let n = quote.leaves.len() as u64;
    assert!(n > 0, "assert_quote_consistent expects a non-empty quote");

    // recoverable_value_sat is exactly the selected leaves' total.
    assert_eq!(
        quote.recoverable_value_sat,
        quote.leaves.iter().map(|l| l.value).sum::<u64>(),
        "recoverable_value_sat must equal the sum of selected leaf values"
    );

    // One funding recommendation per branch, each naming a selected leaf.
    assert_eq!(
        quote.per_branch_funding.len(),
        quote.leaves.len(),
        "one per-branch funding entry per selected leaf"
    );
    let leaf_ids: HashSet<&String> = quote.leaves.iter().map(|l| &l.leaf_id).collect();
    for b in &quote.per_branch_funding {
        assert!(
            leaf_ids.contains(&b.leaf_id),
            "per-branch entry names unselected leaf {}",
            b.leaf_id
        );
        assert!(b.funding_sat > 0, "a per-branch funding amount is zero");
    }

    // A single branch never fans out; multiple branches do at a positive rate.
    if n == 1 {
        assert_eq!(
            quote.fanout_fee_sat, 0,
            "a single branch has no fan-out fee"
        );
    } else if fee_rate_sat_per_vbyte > 0 {
        assert!(
            quote.fanout_fee_sat > 0,
            "multiple branches carry a positive fan-out fee at a positive rate"
        );
    }
    assert!(
        quote.fanout_fee_sat <= quote.total_fee_sat,
        "fan-out fee is part of the total fee"
    );

    // Funding identities from quote_unilateral_exit.
    assert_eq!(
        quote.single_utxo_funding_sat,
        quote
            .per_branch_funding
            .iter()
            .map(|b| b.funding_sat)
            .sum::<u64>()
            + quote.fanout_fee_sat,
        "single_utxo_funding_sat = sum(per_branch) + fanout_fee"
    );
    assert_eq!(
        quote.single_utxo_funding_sat - quote.total_fee_sat,
        n * dust,
        "single_utxo_funding_sat reserves exactly one dust output per branch above fees"
    );

    // The quote echoes the request verbatim.
    assert_eq!(
        quote.fee_rate_sat_per_vbyte, fee_rate_sat_per_vbyte,
        "quote echoes the requested fee rate"
    );
    assert_eq!(
        quote.destination, destination,
        "quote echoes the destination"
    );
}

/// Cross-checks a built exit against the quote it was built from: the value and
/// the leaf set always match (both derive from the same selection). The fee
/// relationship is context-dependent and asserted by the individual tests.
fn assert_build_matches_quote(
    quote: &PrepareUnilateralExitResponse,
    built: &UnilateralExitResponse,
) {
    assert_eq!(
        built.recoverable_value_sat, quote.recoverable_value_sat,
        "built recoverable value must match the quote"
    );
    let mut q: Vec<(&String, u64)> = quote.leaves.iter().map(|l| (&l.leaf_id, l.value)).collect();
    let mut b: Vec<(&String, u64)> = built.leaves.iter().map(|l| (&l.leaf_id, l.value)).collect();
    q.sort();
    b.sort();
    assert_eq!(q, b, "built leaf set must match the quote's");
}

/// Outpoint to value for every output in the set plus the external funding
/// UTXOs, so a transaction's fee is `sum(input values) - sum(outputs)`.
fn output_value_map(
    resp: &UnilateralExitResponse,
    external: &[&FundedUtxo],
) -> Result<HashMap<OutPoint, u64>> {
    let mut map = HashMap::new();
    for entry in &resp.transactions {
        for hex in [Some(&entry.tx_hex), entry.cpfp_tx_hex.as_ref()]
            .into_iter()
            .flatten()
        {
            let tx = decode_tx(hex)?;
            let txid = tx.compute_txid();
            for (vout, out) in tx.output.iter().enumerate() {
                map.insert(
                    OutPoint {
                        txid,
                        vout: u32::try_from(vout)?,
                    },
                    out.value.to_sat(),
                );
            }
        }
    }
    for utxo in external {
        map.insert(utxo.outpoint, utxo.witness_utxo.value.to_sat());
    }
    Ok(map)
}

/// Total value spent by `tx`. `None` if any input's prevout is absent from
/// `map` (an external UTXO not passed in).
fn tx_input_value(tx: &Transaction, map: &HashMap<OutPoint, u64>) -> Option<u64> {
    tx.input
        .iter()
        .map(|i| map.get(&i.previous_output).copied())
        .sum()
}

/// Sum of the CPFP package fees in the set. Computed from the transactions
/// themselves to avoid a sub-dust probe, which bitcoind would reject.
fn sum_package_fees(resp: &UnilateralExitResponse, external: &[&FundedUtxo]) -> Result<u64> {
    let map = output_value_map(resp, external)?;
    let mut total: u64 = 0;
    for entry in &resp.transactions {
        if is_package(entry) {
            let child = decode_tx(entry.cpfp_tx_hex.as_ref().expect("package child"))?;
            let cin = tx_input_value(&child, &map).expect("child input values known");
            let cout: u64 = child.output.iter().map(|o| o.value.to_sat()).sum();
            total = total.saturating_add(cin - cout);
        }
    }
    Ok(total)
}

/// Assert every CPFP package, the fan-out, and the sweep pay at least the target
/// fee rate for their actual weight (the code rounds up, so a package is never
/// below the target). With `near_exact`, packages must also not overpay by more
/// than 1 sat (holds for P2TR, whose witness is fixed-size).
fn assert_fee_rate(
    resp: &UnilateralExitResponse,
    external: &[&FundedUtxo],
    rate: u64,
    near_exact: bool,
) -> Result<()> {
    let map = output_value_map(resp, external)?;
    let target = |weight: u64| weight.saturating_mul(rate).div_ceil(1000);
    for entry in &resp.transactions {
        let (fee, weight) = match entry.kind {
            UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund => {
                let parent = decode_tx(&entry.tx_hex)?;
                let child = decode_tx(
                    entry
                        .cpfp_tx_hex
                        .as_ref()
                        .expect("package carries a CPFP child"),
                )?;
                let child_in = tx_input_value(&child, &map).expect("child input values known");
                let child_out: u64 = child.output.iter().map(|o| o.value.to_sat()).sum();
                (
                    child_in - child_out,
                    parent.weight().to_wu() + child.weight().to_wu(),
                )
            }
            UnilateralExitTxKind::FanOut | UnilateralExitTxKind::Sweep => {
                let tx = decode_tx(&entry.tx_hex)?;
                let tx_in = tx_input_value(&tx, &map).expect("input values known");
                let tx_out: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
                (tx_in - tx_out, tx.weight().to_wu())
            }
        };
        let t = target(weight);
        assert!(
            fee >= t,
            "{:?} fee {fee} is below the target rate ({t} for weight {weight}, rate {rate})",
            entry.kind
        );
        if near_exact
            && matches!(
                entry.kind,
                UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund
            )
        {
            assert!(
                fee <= t + 1,
                "{:?} fee {fee} overpays the target {t} (weight {weight})",
                entry.kind
            );
        }
    }
    Ok(())
}

/// Broadcast a node/refund package (parent `tx_hex` + child `cpfp_tx_hex`) as a
/// 1p1c package, retrying until any relative CSV matures, then mine a block.
async fn broadcast_and_mine(sdk: &LocalSdk, entry: &UnilateralExitTransaction) -> Result<()> {
    let parent = decode_tx(&entry.tx_hex)?;
    let child = decode_tx(
        entry
            .cpfp_tx_hex
            .as_ref()
            .expect("node/refund entry must carry a CPFP child"),
    )?;
    submit_package_with_csv_retry(&sdk.fixtures.bitcoind, &parent, &child).await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;
    Ok(())
}

/// Broadcast and confirm only the fan-out entry, leaving the per-branch chains
/// unbroadcast.
async fn confirm_fan_out(sdk: &LocalSdk, resp: &UnilateralExitResponse) -> Result<String> {
    let fan_out = resp
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::FanOut))
        .expect("a fan-out entry");
    let tx = decode_tx(&fan_out.tx_hex)?;
    sdk.fixtures.bitcoind.broadcast_transaction(&tx).await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;
    Ok(fan_out.txid.clone())
}

/// Broadcast and mine the entire built exit in list (topological) order:
/// node/refund entries as 1p1c packages, the fan-out and the sweep on their own.
/// Then assert every transaction is retrievable from a block (mined, not merely
/// accepted) and that the sweep pays `destination`. This is the "the built set
/// can actually be mined" check.
async fn assert_all_mined(
    sdk: &LocalSdk,
    built: &UnilateralExitResponse,
    destination: &Address,
) -> Result<()> {
    let mut sweep_txid: Option<Txid> = None;
    for entry in &built.transactions {
        match entry.kind {
            UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund => {
                broadcast_and_mine(sdk, entry).await?;
            }
            UnilateralExitTxKind::FanOut => {
                let tx = decode_tx(&entry.tx_hex)?;
                sdk.fixtures.bitcoind.broadcast_transaction(&tx).await?;
                sdk.fixtures.bitcoind.generate_blocks(1).await?;
            }
            UnilateralExitTxKind::Sweep => {
                let tx = decode_tx(&entry.tx_hex)?;
                let txid = sdk.fixtures.bitcoind.broadcast_transaction(&tx).await?;
                sdk.fixtures.bitcoind.generate_blocks(1).await?;
                sweep_txid = Some(txid);
            }
        }
    }

    // Every entry now reads back from a block: bitcoind mined it.
    for entry in &built.transactions {
        let txid = Txid::from_str(&entry.txid)?;
        let mined = sdk.fixtures.bitcoind.get_transaction(&txid).await?;
        assert_eq!(
            mined.compute_txid(),
            txid,
            "{:?} {} was not mined",
            entry.kind,
            entry.txid
        );
    }

    let sweep_txid = sweep_txid.expect("the set terminates in a sweep");
    let sweep = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    assert!(
        sweep
            .output
            .iter()
            .any(|o| o.script_pubkey == destination.script_pubkey()),
        "the sweep must pay the destination"
    );
    Ok(())
}

/// A fresh single-UTXO auto exit quote and its build, funded by one P2TR UTXO of
/// `funding_sat`. Returns the quote, the built set, and the funding UTXO so a
/// test can assert on all three.
async fn quote_then_build_single(
    sdk: &LocalSdk,
    funding_sat: u64,
    fee_rate_sat_per_vbyte: u64,
) -> Result<(
    PrepareUnilateralExitResponse,
    UnilateralExitResponse,
    FundedUtxo,
)> {
    let utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(funding_sat)).await?;
    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte,
            funding_kind: CpfpFundingKind::P2tr,
            destination: utxo.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let signer = signer_for(&utxo.secret_key.secret_bytes())?;
    let built = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&utxo)],
            },
            signer,
        )
        .await?;
    Ok((quote, built, utxo))
}

/// Nothing on-chain yet: the quote is internally consistent, the build matches
/// it, every node/refund entry is unconfirmed and carries a CPFP child, and a
/// single sweep terminates the set.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_nothing_confirmed(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let destination = cpfp.address.to_string();

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(quote.leaves.len(), 1, "expected a single selected leaf");
    assert!(quote.recoverable_value_sat > 0);
    assert!(quote.total_fee_sat > 0);
    assert_quote_consistent(&quote, FEE_RATE, &destination, p2tr_dust());

    let signer = signer_for(&cpfp.secret_key.secret_bytes())?;
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);

    let sweeps: Vec<_> = resp
        .transactions
        .iter()
        .filter(|t| matches!(t.kind, UnilateralExitTxKind::Sweep))
        .collect();
    assert_eq!(sweeps.len(), 1, "exactly one sweep expected");
    assert!(sweeps[0].cpfp_tx_hex.is_none(), "sweep has no CPFP child");
    assert!(
        matches!(
            resp.transactions.last().map(|t| &t.kind),
            Some(UnilateralExitTxKind::Sweep)
        ),
        "sweep must be last"
    );

    let packages: Vec<_> = resp.transactions.iter().filter(|t| is_package(t)).collect();
    assert!(!packages.is_empty(), "expected node/refund packages");
    for entry in &packages {
        assert!(
            entry.cpfp_tx_hex.is_some(),
            "unconfirmed package must carry a CPFP child"
        );
        assert!(matches!(entry.status, ConfirmationStatus::Unconfirmed));
    }
    assert!(
        resp.transactions
            .iter()
            .any(|t| t.csv_timelock_blocks.is_some_and(|b| b > 0)),
        "at least one entry (the refund) carries a positive CSV timelock"
    );
    Ok(())
}

/// A full single-leaf exit is broadcast and mined end to end: every transaction
/// lands in a block and the sweep pays the destination. The build reproduces the
/// quote's value, leaf set, and (fresh, single-UTXO) total fee exactly.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_full_exit_and_sweep(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let destination = cpfp.address.clone();

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_quote_consistent(&quote, FEE_RATE, &destination.to_string(), p2tr_dust());

    let signer = signer_for(&cpfp.secret_key.secret_bytes())?;
    let built = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer,
        )
        .await?;
    assert_build_matches_quote(&quote, &built);
    assert_eq!(
        built.total_fee_sat, quote.total_fee_sat,
        "a fresh single-UTXO build reproduces the quote's total fee exactly"
    );

    assert_all_mined(&sdk, &built, &destination).await?;

    // Value conservation, the feature's core guarantee: the funding UTXO plus the
    // recovered leaf value, minus the exact on-chain fee, lands at the
    // destination. P2TR signatures are fixed size, so the fee is exact and the
    // swept amount equals the quote with no slack.
    let sweep_txid = built
        .transactions
        .iter()
        .find(|t| t.kind == UnilateralExitTxKind::Sweep)
        .map(|t| Txid::from_str(&t.txid))
        .expect("the built set terminates in a sweep")?;
    let sweep = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    let swept = sweep
        .output
        .iter()
        .find(|o| o.script_pubkey == destination.script_pubkey())
        .map(|o| o.value.to_sat())
        .expect("the sweep pays the destination");
    let expected = CPFP_SATS + built.recoverable_value_sat - built.total_fee_sat;
    assert_eq!(
        swept, expected,
        "swept {swept} must equal funding ({CPFP_SATS}) + recoverable \
         ({}) - total fee ({})",
        built.recoverable_value_sat, built.total_fee_sat
    );
    Ok(())
}

/// Re-running a completed exit re-drives nothing. `Auto` drops the exited leaf
/// from the available set once the exit is mined, but it is still sourceable by
/// id, so forcing it back in with `Specific` runs the build rather than the
/// empty-plan early return. Its refund address is then funded with no unspent
/// output, so the exit resolves the refund as already-swept: it rebuilds no
/// refund and re-attempts no sweep. Under the pre-fix behavior the empty address
/// scan would re-drive the refund (with a fresh CPFP child) and rebuild the sweep.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_completed_exit_rerun_redrives_nothing(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let destination = cpfp.address.clone();

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    // The wallet drops the leaf from its available set once the exit is mined, so
    // capture its id now to force it back in on the retry.
    let exited_leaf_ids: Vec<String> = quote.leaves.iter().map(|l| l.leaf_id.clone()).collect();
    let built = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    assert_all_mined(&sdk, &built, &destination).await?;
    let recoverable = built.recoverable_value_sat;

    // The first pass spent the CPFP UTXO chain, so fund a fresh one for the retry.
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let rerun_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Specific {
                leaf_ids: exited_leaf_ids,
            },
        })
        .await?;
    let rerun = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: rerun_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    // The leaf is re-selected (recoverable value is unchanged), so the build ran
    // rather than the empty-plan early return, yet it rebuilds no refund and
    // re-attempts no sweep: the swept refund was recognized.
    assert_eq!(
        rerun.recoverable_value_sat, recoverable,
        "the forced-in leaf is re-selected, so the build runs the swept-refund path"
    );
    assert!(
        rerun.transactions.iter().all(|t| !matches!(
            t.kind,
            UnilateralExitTxKind::Refund | UnilateralExitTxKind::Sweep
        )),
        "a completed exit rebuilds no refund and re-attempts no sweep"
    );
    assert!(
        rerun.transactions.iter().all(|t| t.cpfp_tx_hex.is_none()),
        "no fresh CPFP child: nothing needs broadcasting on a completed exit"
    );
    Ok(())
}

/// Confirming the first package then re-preparing brings it back `Confirmed`
/// with no CPFP child, while later entries stay unconfirmed with their children.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_first_package_confirmed_resumes(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    let first_pkg = first
        .transactions
        .iter()
        .find(|t| is_package(t))
        .expect("a node/refund package");
    let confirmed_txid = first_pkg.txid.clone();
    broadcast_and_mine(&sdk, first_pkg).await?;

    // The first pass spent our CPFP UTXO chain, so fund a fresh one.
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let second_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: second_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    let resumed = second
        .transactions
        .iter()
        .find(|t| t.txid == confirmed_txid)
        .expect("the confirmed package must still appear");
    assert!(matches!(resumed.status, ConfirmationStatus::Confirmed));
    assert!(
        resumed.cpfp_tx_hex.is_none(),
        "a confirmed step carries no CPFP child"
    );
    assert!(
        second
            .transactions
            .iter()
            .any(|t| is_package(t) && matches!(t.status, ConfirmationStatus::Unconfirmed)),
        "later packages remain unconfirmed"
    );
    Ok(())
}

/// The sweep opts into RBF (BIP125): every input signals it, and once broadcast
/// with confirmed ancestors bitcoind reports the sweep as `bip125-replaceable`.
/// So an under-fee sweep sitting unconfirmed can be replaced by a higher-fee
/// rebuild. bitcoind rejects replacement of non-signaling txs by default, so
/// this would fail if the sweep inputs fell back to the default (non-RBF) sequence.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_sweep_is_rbf_replaceable(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    // Captured before the quote is moved: the mined leaf drops out of the available
    // set, so the higher-rate re-quote must force it back in by id.
    let leaf_ids: Vec<String> = quote.leaves.iter().map(|l| l.leaf_id.clone()).collect();
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    // Confirm everything up to the sweep so it has no unconfirmed ancestors; then
    // its reported replaceability reflects only the sweep's own signaling.
    let mut sweep: Option<Transaction> = None;
    for entry in &resp.transactions {
        match entry.kind {
            UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund => {
                broadcast_and_mine(&sdk, entry).await?;
            }
            UnilateralExitTxKind::FanOut => {
                let tx = decode_tx(&entry.tx_hex)?;
                sdk.fixtures.bitcoind.broadcast_transaction(&tx).await?;
                sdk.fixtures.bitcoind.generate_blocks(1).await?;
            }
            UnilateralExitTxKind::Sweep => sweep = Some(decode_tx(&entry.tx_hex)?),
        }
    }
    let sweep = sweep.expect("a sweep transaction");

    // Structural: every sweep input opts into RBF.
    for (i, input) in sweep.input.iter().enumerate() {
        assert!(
            input.sequence.is_rbf(),
            "sweep input {i} must signal RBF, got sequence {:#010x}",
            input.sequence.0
        );
    }

    // Policy: broadcast (without mining) and confirm bitcoind sees it replaceable.
    let sweep_txid = sdk.fixtures.bitcoind.broadcast_transaction(&sweep).await?;
    let entry: serde_json::Value = sdk
        .fixtures
        .bitcoind
        .rpc(
            "getmempoolentry",
            &[serde_json::json!(sweep_txid.to_string())],
        )
        .await?;
    assert_eq!(
        entry
            .get("bip125-replaceable")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "bitcoind must report the sweep as BIP125-replaceable: {entry}"
    );

    // Now actually replace it: re-quote the same exit at a higher fee rate and
    // rebuild. The refund is a fixed pre-signed tx confirmed on-chain, so the
    // rebuild spends the same refund outpoint (a higher fee just shrinks the
    // output), conflicting with the pending sweep. The mined leaf is no longer
    // available, so force it back in by id.
    //
    // +2 sat/vByte, not +1: the rebuild adopts the confirmed refund and drops the
    // original sweep's terminal CPFP-change input, so the replacement is a smaller
    // one-input tx. BIP125 rule 4 makes it pay its own vsize over the original fee,
    // which a single-sat bump on the smaller tx cannot cover.
    let higher_rate = FEE_RATE + 2;
    let bump_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: higher_rate,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Specific { leaf_ids },
        })
        .await?;
    let bumped = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: bump_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    let higher_sweep = bumped
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Sweep))
        .map(|t| decode_tx(&t.tx_hex))
        .expect("the rebuild re-emits a sweep over the pending refund")?;

    // A distinct transaction that conflicts with the original on a shared input.
    assert_ne!(
        higher_sweep.compute_txid(),
        sweep_txid,
        "the higher-fee sweep must be a different transaction"
    );
    let original_inputs: HashSet<OutPoint> =
        sweep.input.iter().map(|i| i.previous_output).collect();
    assert!(
        higher_sweep
            .input
            .iter()
            .any(|i| original_inputs.contains(&i.previous_output)),
        "the replacement must spend one of the original sweep's inputs"
    );

    // Broadcast the replacement: bitcoind accepts it and evicts the original.
    let higher_txid = sdk
        .fixtures
        .bitcoind
        .broadcast_transaction(&higher_sweep)
        .await?;
    let mempool: Vec<String> = sdk.fixtures.bitcoind.rpc("getrawmempool", &[]).await?;
    assert!(
        mempool.contains(&higher_txid.to_string()),
        "the higher-fee replacement must be in the mempool: {mempool:?}"
    );
    assert!(
        !mempool.contains(&sweep_txid.to_string()),
        "the original sweep must be evicted by its replacement: {mempool:?}"
    );
    Ok(())
}

// Every multi-leaf test here builds its leaves from separate deposits, i.e.
// independent single-leaf trees with no shared ancestor. The shared-ancestor
// case (two leaves under one intermediate: the emitted-once dedup and the
// dependency threading across a skipped shared ancestor) is unit-tested in
// spark-wallet `exit_build_tests::build_dedups_shared_ancestors_and_threads_dependencies`.
// TODO: add an integration test that exits two leaves sharing a real on-chain
// ancestor once the fixture can split one deposit into multiple leaves (needs
// the SSP / a tree-split op).

/// Two leaves funded by a single CPFP input, driven end to end: the quote reports a
/// positive fan-out fee and selects both leaves, the build emits a fan-out, and the
/// whole set (fan-out, both branches, sweep) mines, with the funding UTXO plus both
/// recovered leaves landing at the destination minus the exact on-chain fee.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_multi_leaf_fan_out_and_sweep(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let funding_sat = CPFP_SATS * 4;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(funding_sat)).await?;
    let destination = cpfp.address.clone();

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(quote.leaves.len(), 2, "expected two selected leaves");
    assert!(
        quote.fanout_fee_sat > 0,
        "a single funding input across two branches has a fan-out fee"
    );
    assert_quote_consistent(&quote, FEE_RATE, &destination.to_string(), p2tr_dust());

    let built = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &built);
    assert!(
        built
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "a single funding input across two branches must produce a fan-out"
    );

    // Drive the whole set on-chain: the fan-out, both branches (each node and its
    // refund with a CPFP child), and the single sweep that folds both leaves.
    assert_all_mined(&sdk, &built, &destination).await?;

    // Value conservation across both branches: the funding UTXO plus both recovered
    // leaves, minus the exact on-chain fee, lands at the destination. P2TR
    // signatures are fixed size, so the fee is exact and there is no slack.
    let sweep_txid = built
        .transactions
        .iter()
        .find(|t| t.kind == UnilateralExitTxKind::Sweep)
        .map(|t| Txid::from_str(&t.txid))
        .expect("the built set terminates in a sweep")?;
    let sweep = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    let swept = sweep
        .output
        .iter()
        .find(|o| o.script_pubkey == destination.script_pubkey())
        .map(|o| o.value.to_sat())
        .expect("the sweep pays the destination");
    let expected = funding_sat + built.recoverable_value_sat - built.total_fee_sat;
    assert_eq!(
        swept, expected,
        "swept {swept} must equal funding ({funding_sat}) + recoverable \
         ({}) - total fee ({})",
        built.recoverable_value_sat, built.total_fee_sat
    );
    Ok(())
}

/// The feature's core promise: with the operators offline, a wallet still exits.
/// Two leaves are claimed and synced while the operators are up, persisting each
/// leaf's full exit chain to the durable tree store. The operators are then
/// stopped, and the exit is prepared, built, and driven fully on-chain sourcing
/// every chain from local storage alone. Broadcasting goes to bitcoind, not the
/// operators, so an offline exit confirms and sweeps like an online one.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_multi_leaf_offline_exit(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;

    // Wait for both claimed leaves to finalize to Available and sync into the
    // durable tree store. This needs the operators (the Creating -> Available
    // transition and the leaf download), so it must complete before they stop;
    // afterwards the exit sources everything it needs from local storage.
    let balance = wait_for_balance(&sdk.sdk, Some(LEAF_SATS * 2), None, 60).await?;
    assert_eq!(
        balance,
        LEAF_SATS * 2,
        "both claimed leaves are synced into the durable store"
    );

    let funding_sat = CPFP_SATS * 4;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(funding_sat)).await?;
    let destination = cpfp.address.clone();

    // Take the operators offline. Everything below must source from local storage.
    sdk.fixtures.stop_operators().await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(
        quote.leaves.len(),
        2,
        "both persisted leaves are exitable with the operators offline"
    );
    assert_quote_consistent(&quote, FEE_RATE, &destination.to_string(), p2tr_dust());

    let built = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &built);

    // The whole set lands on-chain with the operators still offline: the fan-out,
    // both branches (each node and its refund with a CPFP child), and the sweep.
    assert_all_mined(&sdk, &built, &destination).await?;

    // Value conservation: the funding UTXO plus both recovered leaves, minus the
    // exact on-chain fee, lands at the destination. P2TR signatures are fixed
    // size, so the fee is exact and there is no slack.
    let sweep_txid = built
        .transactions
        .iter()
        .find(|t| t.kind == UnilateralExitTxKind::Sweep)
        .map(|t| Txid::from_str(&t.txid))
        .expect("the built set terminates in a sweep")?;
    let sweep = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    let swept = sweep
        .output
        .iter()
        .find(|o| o.script_pubkey == destination.script_pubkey())
        .map(|o| o.value.to_sat())
        .expect("the sweep pays the destination");
    let expected = funding_sat + built.recoverable_value_sat - built.total_fee_sat;
    assert_eq!(
        swept, expected,
        "offline exit conserves value: swept {swept} = funding ({funding_sat}) \
         + recoverable ({}) - fee ({})",
        built.recoverable_value_sat, built.total_fee_sat
    );
    Ok(())
}

/// Re-preparing at the same fee rate after a fan-out confirms adopts that
/// fan-out in place (same txid, `Confirmed`, no child) rather than rebuilding
/// it, and both leaves remain exitable through its outputs.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_confirmed_fan_out_is_adopted(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS * 4)).await?;
    let funding = vec![cpfp_input(&cpfp)];
    let key = cpfp.secret_key.secret_bytes();
    let dest = cpfp.address.to_string();

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: funding.clone(),
            },
            signer_for(&key)?,
        )
        .await?;
    let fan_out_txid = confirm_fan_out(&sdk, &first).await?;

    // Re-prepare with the same funding outpoint (now spent by the confirmed
    // fan-out) at the same fee rate.
    let second_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: second_quote,
                funding_inputs: funding,
            },
            signer_for(&key)?,
        )
        .await?;
    let adopted = second
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::FanOut))
        .expect("the confirmed fan-out must still appear");
    assert_eq!(
        adopted.txid, fan_out_txid,
        "adopts the confirmed fan-out, not a fresh one"
    );
    assert!(matches!(adopted.status, ConfirmationStatus::Confirmed));
    assert!(
        adopted.cpfp_tx_hex.is_none(),
        "a fan-out carries no CPFP child"
    );
    assert_eq!(
        second.leaves.len(),
        2,
        "both leaves remain exitable through the adopted fan-out"
    );
    Ok(())
}

/// After a fan-out confirms at a modest rate, re-preparing at a much higher rate
/// can't be funded from the original UTXO and returns `InsufficientCpfpFunds`.
/// Recovery by re-funding is covered in `test_higher_rate_recovers_by_refunding`.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_confirmed_fan_out_insufficient_at_higher_fee(
    #[case] backend: SignerBackend,
) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    // Modest funding: enough for a fan-out at FEE_RATE, but far too little for a
    // fresh fan-out at 40x the rate.
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(20_000)).await?;
    let funding = vec![cpfp_input(&cpfp)];
    let key = cpfp.secret_key.secret_bytes();
    let dest = cpfp.address.to_string();

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: funding.clone(),
            },
            signer_for(&key)?,
        )
        .await?;
    confirm_fan_out(&sdk, &first).await?;

    let high_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE * 40,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest,
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let err = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: high_quote,
                funding_inputs: funding,
            },
            signer_for(&key)?,
        )
        .await
        .expect_err("a 40x fee cannot be funded from the original UTXO");
    assert!(
        matches!(err, SdkError::InsufficientCpfpFunds { .. }),
        "expected InsufficientCpfpFunds, got: {err:?}"
    );
    Ok(())
}

/// Fee exactness at 1 sat/vB: the quote is consistent, and every built CPFP
/// package and the sweep pay exactly the target rate (rounded up) for P2TR
/// funding. The fresh single-UTXO build fee equals the quote fee.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_fees_exact_at_1_sat_per_vb(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let (quote, resp, cpfp) = quote_then_build_single(&sdk, CPFP_SATS, FEE_RATE).await?;
    assert_quote_consistent(&quote, FEE_RATE, &cpfp.address.to_string(), p2tr_dust());
    assert_eq!(resp.total_fee_sat, quote.total_fee_sat);
    assert_fee_rate(&resp, &[&cpfp], FEE_RATE_KW, true)?;
    Ok(())
}

/// Fee exactness at a higher rate (3 sat/vByte == 750 sat/kW) proves the fee is
/// the ceiling of weight x rate / 1000, not a floor or round, and that it scales
/// with the requested rate.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_fees_round_up_at_3_sat_per_vb(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let (quote, resp, cpfp) = quote_then_build_single(&sdk, CPFP_SATS, 3).await?;
    assert_quote_consistent(&quote, 3, &cpfp.address.to_string(), p2tr_dust());
    assert_eq!(resp.total_fee_sat, quote.total_fee_sat);
    assert_fee_rate(&resp, &[&cpfp], 3 * 250, true)?;
    Ok(())
}

/// P2WPKH funding: the quote is consistent (P2WPKH dust reserve), and fees never
/// fall below the target rate (the declared input weight covers the worst-case
/// DER signature, so the package may overpay by a sat but never underpays).
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_fees_p2wpkh_never_below_rate(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2wpkh_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let dust = cpfp.witness_utxo.script_pubkey.minimal_non_dust().to_sat();

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2wpkh,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_quote_consistent(&quote, FEE_RATE, &cpfp.address.to_string(), dust);

    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input_for(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    // total_fee_sat now reports the fee the built txs actually pay, yet it still
    // equals the quote: the amount held back in each child is fixed by the
    // worst-case input weight, so a P2WPKH witness that comes in a sat under that
    // worst case raises the effective rate, not the sat amount paid.
    assert_eq!(resp.total_fee_sat, quote.total_fee_sat);
    assert_fee_rate(&resp, &[&cpfp], FEE_RATE_KW, false)?;
    Ok(())
}

/// A single leaf funded by several UTXOs: no fan-out, and the first CPFP child
/// spends them all.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_single_leaf_multiple_utxos(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let key = fixed_key(0x21);
    let bitcoind = &sdk.fixtures.bitcoind;
    let u1 = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS / 2), &key).await?;
    let u2 = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS / 2), &key).await?;
    let u3 = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS / 2), &key).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: u1.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_quote_consistent(&quote, FEE_RATE, &u1.address.to_string(), p2tr_dust());

    let funding = vec![
        cpfp_input_for(&u1),
        cpfp_input_for(&u2),
        cpfp_input_for(&u3),
    ];
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: funding,
            },
            signer_for(&key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert_eq!(resp.leaves.len(), 1);
    assert!(
        !resp
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "a single leaf never fans out"
    );
    Ok(())
}

/// Two leaves, two UTXOs each large enough for a branch: 1:1 assignment, no
/// fan-out. The quote plans for the single-UTXO fan-out (positive fanout_fee),
/// but funding one sufficient UTXO per branch avoids it, so the built set
/// carries no fan-out transaction. (The per-branch build's actual fee is not
/// the quote's `total_fee - fanout_fee`: the quote sizes every branch's first
/// CPFP child off one representative input, while the build sizes it off the
/// branch's real inputs, so the two costs only approximately agree.)
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_two_leaves_two_utxos_no_fanout(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let key = fixed_key(0x22);
    let bitcoind = &sdk.fixtures.bitcoind;
    let u1 = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS), &key).await?;
    let u2 = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS), &key).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: u1.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(quote.leaves.len(), 2);
    assert!(
        quote.fanout_fee_sat > 0,
        "the quote plans a single-UTXO fan-out"
    );
    assert_quote_consistent(&quote, FEE_RATE, &u1.address.to_string(), p2tr_dust());

    let funding = vec![cpfp_input_for(&u1), cpfp_input_for(&u2)];
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: funding,
            },
            signer_for(&key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert!(
        !resp
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "two sufficient UTXOs fund the two branches 1:1, no fan-out"
    );
    Ok(())
}

/// Two leaves, two UTXOs where one is too small for its branch and there are not
/// enough inputs to combine, so the exit falls back to a fan-out.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_two_leaves_undersized_utxo_fans_out(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let key = fixed_key(0x23);
    let bitcoind = &sdk.fixtures.bitcoind;
    let big = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS), &key).await?;
    let tiny = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(500), &key).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: big.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(quote.leaves.len(), 2);

    let funding = vec![cpfp_input_for(&big), cpfp_input_for(&tiny)];
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: funding,
            },
            signer_for(&key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert!(
        resp.transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "an undersized UTXO with no room to combine forces a fan-out"
    );
    Ok(())
}

/// Two leaves, three UTXOs: one branch covered by a single UTXO, the other by
/// two combined. Subset assignment avoids a fan-out.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_two_leaves_subset_assignment_no_fanout(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let key = fixed_key(0x24);
    let bitcoind = &sdk.fixtures.bitcoind;
    let big = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS), &key).await?;
    let a = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(3_000), &key).await?;
    let b = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(3_000), &key).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: big.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(quote.leaves.len(), 2);

    let funding = vec![cpfp_input_for(&big), cpfp_input_for(&a), cpfp_input_for(&b)];
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: funding,
            },
            signer_for(&key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert!(
        !resp
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "the three UTXOs partition across the two branches, so no fan-out is needed \
         (the combining logic itself is unit-tested in the planner)"
    );
    Ok(())
}

/// A generously-funded fan-out carries per-branch headroom, so re-preparing at a
/// higher rate re-adopts the confirmed fan-out (no re-funding needed).
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_higher_rate_reuses_fan_out_within_headroom(
    #[case] backend: SignerBackend,
) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS * 4)).await?;
    let funding = vec![cpfp_input(&cpfp)];
    let key = cpfp.secret_key.secret_bytes();
    let dest = cpfp.address.to_string();

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: funding.clone(),
            },
            signer_for(&key)?,
        )
        .await?;
    let fan_out_txid = confirm_fan_out(&sdk, &first).await?;

    // Twice the rate still fits within the generous headroom.
    let second_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE * 2,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest,
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: second_quote,
                funding_inputs: funding,
            },
            signer_for(&key)?,
        )
        .await?;
    let fan = second
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::FanOut))
        .expect("the confirmed fan-out is reused");
    assert_eq!(fan.txid, fan_out_txid, "adopts the same confirmed fan-out");
    assert!(matches!(fan.status, ConfirmationStatus::Confirmed));
    assert_eq!(second.leaves.len(), 2);
    Ok(())
}

/// When a higher rate cannot be funded from the original UTXO, the caller
/// recovers by passing the confirmed fan-out's outputs (plus extra funding) as
/// inputs: a fresh fan-out at the higher rate then succeeds.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_higher_rate_recovers_by_refunding(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let key = fixed_key(0x25);
    let key_bytes = key.secret_bytes();
    let bitcoind = &sdk.fixtures.bitcoind;
    let cpfp = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(20_000), &key).await?;
    let dest = cpfp.address.to_string();
    let pubkey = hex::encode(key.public_key(&bitcoin::key::Secp256k1::new()).serialize());

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: vec![cpfp_input_for(&cpfp)],
            },
            signer_for(&key_bytes)?,
        )
        .await?;
    let fan_out_txid = confirm_fan_out(&sdk, &first).await?;

    let high_rate = FEE_RATE * 40;
    let high_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: high_rate,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest.clone(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let err = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: high_quote,
                funding_inputs: vec![cpfp_input_for(&cpfp)],
            },
            signer_for(&key_bytes)?,
        )
        .await
        .expect_err("the higher rate can't be funded from the original UTXO");
    assert!(matches!(err, SdkError::InsufficientCpfpFunds { .. }));

    // Recovery: spend the confirmed fan-out's outputs plus a fresh larger UTXO.
    let fan_entry = first
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::FanOut))
        .expect("a fan-out entry");
    let fan_tx = decode_tx(&fan_entry.tx_hex)?;
    let extra = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS * 4), &key).await?;
    let mut funding = vec![cpfp_input_for(&extra)];
    for (vout, out) in fan_tx.output.iter().enumerate() {
        funding.push(CpfpInput::P2tr {
            txid: fan_out_txid.clone(),
            vout: u32::try_from(vout)?,
            value: out.value.to_sat(),
            pubkey: pubkey.clone(),
        });
    }
    let recovery_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: high_rate,
            funding_kind: CpfpFundingKind::P2tr,
            destination: dest,
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let recovered = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: recovery_quote,
                funding_inputs: funding,
            },
            signer_for(&key_bytes)?,
        )
        .await?;
    assert_eq!(recovered.leaves.len(), 2);
    assert!(
        recovered
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "the recovery builds a fresh fan-out at the higher rate"
    );
    Ok(())
}

/// Auto selection at an astronomically high rate finds no profitable leaf (the
/// exit cost exceeds every leaf's value): the quote is empty (no leaves, all fee
/// fields zero) and building it yields no transactions rather than erroring.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_no_profitable_leaves_auto(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE * 500,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert!(
        quote.leaves.is_empty(),
        "expected no leaves at 500x the base rate, got: {:?}",
        quote.leaves
    );
    assert_eq!(quote.recoverable_value_sat, 0);
    assert_eq!(quote.total_fee_sat, 0);
    assert_eq!(quote.fanout_fee_sat, 0);
    assert_eq!(quote.single_utxo_funding_sat, 0);
    assert!(quote.per_branch_funding.is_empty());

    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await
        .expect("an empty quote builds to an empty set, not an error");
    assert!(resp.leaves.is_empty());
    assert!(
        resp.transactions.is_empty(),
        "an empty exit must carry no transactions"
    );
    Ok(())
}

/// Specific selection exits exactly the named leaf, ignoring other leaves added
/// to the wallet in the meantime.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_specific_ignores_other_leaves(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let first_id = sdk.spark_wallet.list_leaves().await?.available[0]
        .id
        .to_string();
    // A second leaf appears after the first is chosen.
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Specific {
                leaf_ids: vec![first_id.clone()],
            },
        })
        .await?;
    assert_eq!(quote.leaves.len(), 1, "only the named leaf is exited");
    assert_eq!(quote.leaves[0].leaf_id, first_id);
    assert_quote_consistent(&quote, FEE_RATE, &cpfp.address.to_string(), p2tr_dust());

    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert_eq!(resp.leaves[0].leaf_id, first_id);
    Ok(())
}

/// Preparing twice with identical parameters (nothing broadcast) yields an
/// identical quote, and building each yields an identical transaction set: both
/// phases are deterministic.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_prepare_is_idempotent(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let funding = vec![cpfp_input(&cpfp)];
    let key = cpfp.secret_key.secret_bytes();
    let request = || PrepareUnilateralExitRequest {
        fee_rate_sat_per_vbyte: FEE_RATE,
        funding_kind: CpfpFundingKind::P2tr,
        destination: cpfp.address.to_string(),
        selection: ExitLeafSelection::Auto,
    };

    let quote_a = sdk.sdk.prepare_unilateral_exit(request()).await?;
    let quote_b = sdk.sdk.prepare_unilateral_exit(request()).await?;
    assert_eq!(
        (
            quote_a.recoverable_value_sat,
            quote_a.total_fee_sat,
            quote_a.fanout_fee_sat,
            quote_a.single_utxo_funding_sat
        ),
        (
            quote_b.recoverable_value_sat,
            quote_b.total_fee_sat,
            quote_b.fanout_fee_sat,
            quote_b.single_utxo_funding_sat
        ),
        "identical requests produce identical quote figures"
    );

    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote_a,
                funding_inputs: funding.clone(),
            },
            signer_for(&key)?,
        )
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote_b,
                funding_inputs: funding,
            },
            signer_for(&key)?,
        )
        .await?;
    let a: Vec<&String> = first.transactions.iter().map(|t| &t.txid).collect();
    let b: Vec<&String> = second.transactions.iter().map(|t| &t.txid).collect();
    assert_eq!(a, b, "identical params produce identical txids");
    Ok(())
}

/// Preparing at a higher rate quotes a higher fee, and building it replaces the
/// fee-bearing transactions: the sweep pays a different fee, so its txid changes
/// (RBF).
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_higher_rate_changes_sweep_txid(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let funding = vec![cpfp_input(&cpfp)];
    let key = cpfp.secret_key.secret_bytes();
    let request = |rate| PrepareUnilateralExitRequest {
        fee_rate_sat_per_vbyte: rate,
        funding_kind: CpfpFundingKind::P2tr,
        destination: cpfp.address.to_string(),
        selection: ExitLeafSelection::Auto,
    };
    let sweep_txid = |resp: &UnilateralExitResponse| {
        resp.transactions
            .iter()
            .find(|t| matches!(t.kind, UnilateralExitTxKind::Sweep))
            .map(|t| t.txid.clone())
            .expect("a sweep")
    };

    let low_quote = sdk.sdk.prepare_unilateral_exit(request(FEE_RATE)).await?;
    let high_quote = sdk
        .sdk
        .prepare_unilateral_exit(request(FEE_RATE * 3))
        .await?;
    assert!(
        high_quote.total_fee_sat > low_quote.total_fee_sat,
        "a higher rate quotes a higher total fee"
    );

    let low = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: low_quote,
                funding_inputs: funding.clone(),
            },
            signer_for(&key)?,
        )
        .await?;
    let high = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: high_quote,
                funding_inputs: funding,
            },
            signer_for(&key)?,
        )
        .await?;
    assert_ne!(
        sweep_txid(&low),
        sweep_txid(&high),
        "a higher rate re-signs the sweep at a different fee"
    );
    Ok(())
}

/// Empty funding is rejected by the build.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_empty_funding_rejected(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let err = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await
        .expect_err("no funding inputs");
    assert!(matches!(err, SdkError::InvalidInput(_)), "got: {err:?}");
    Ok(())
}

/// An explicit selection with no leaf ids is rejected up front by prepare.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_explicit_empty_list_rejected(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let err = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Specific { leaf_ids: vec![] },
        })
        .await
        .expect_err("no leaves named");
    assert!(matches!(err, SdkError::InvalidInput(_)), "got: {err:?}");
    Ok(())
}

/// A zero fee rate is accepted: the quote's total fee is zero and every built
/// transaction carries no fee.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_zero_fee_rate_succeeds(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: 0,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_eq!(quote.total_fee_sat, 0, "a zero fee rate quotes a zero fee");
    assert_quote_consistent(&quote, 0, &cpfp.address.to_string(), p2tr_dust());

    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    assert_eq!(
        resp.total_fee_sat, 0,
        "a zero fee rate produces zero-fee txs"
    );
    assert_eq!(resp.leaves.len(), 1);
    Ok(())
}

/// One leaf funded by a P2TR and a P2WPKH UTXO sharing a key: the single-key
/// signer signs both input types.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_mixed_p2tr_p2wpkh_funding(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let key = fixed_key(0x31);
    let bitcoind = &sdk.fixtures.bitcoind;
    let taproot = fund_p2tr_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS), &key).await?;
    let segwit = fund_p2wpkh_utxo_with_key(bitcoind, Amount::from_sat(CPFP_SATS), &key).await?;

    // The quote is sized for the first (P2TR) input kind.
    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: taproot.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_quote_consistent(&quote, FEE_RATE, &taproot.address.to_string(), p2tr_dust());

    let funding = vec![cpfp_input_for(&taproot), cpfp_input_for(&segwit)];
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: funding,
            },
            signer_for(&key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert!(
        resp.transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::Sweep)),
        "the mixed-funding exit still produces a sweep"
    );
    Ok(())
}

/// Single leaf, single UTXO: the quote's `single_utxo_funding_sat` carries
/// sweep-fee headroom above the hard build minimum (`sum(package fees) + dust`).
/// Funding that recommendation builds; funding exactly the hard minimum still
/// builds; one sat below it returns `InsufficientCpfpFunds` reporting that
/// minimum.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_single_leaf_funding_boundary(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;

    // A generous probe: get the quote and the actual package fees.
    let (quote, probe, generous) = quote_then_build_single(&sdk, CPFP_SATS, FEE_RATE).await?;
    let dust = generous
        .witness_utxo
        .script_pubkey
        .minimal_non_dust()
        .to_sat();
    assert_quote_consistent(&quote, FEE_RATE, &generous.address.to_string(), dust);
    // The hard build minimum for one leaf: CPFP package fees + one dust reserve.
    let minimum = sum_package_fees(&probe, &[&generous])? + dust;
    assert!(
        quote.single_utxo_funding_sat > minimum,
        "the single-UTXO recommendation ({}) reserves sweep-fee headroom above the hard minimum ({minimum})",
        quote.single_utxo_funding_sat
    );

    // The quote's recommendation funds the exit.
    let (_, recommended, _) =
        quote_then_build_single(&sdk, quote.single_utxo_funding_sat, FEE_RATE).await?;
    assert_eq!(recommended.leaves.len(), 1);

    // Exactly the hard minimum funds it; one sat less does not.
    let (_, exact, _) = quote_then_build_single(&sdk, minimum, FEE_RATE).await?;
    assert_eq!(exact.leaves.len(), 1, "the hard minimum funds the exit");

    let short = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(minimum - 1)).await?;
    let short_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: short.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let err = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: short_quote,
                funding_inputs: vec![cpfp_input(&short)],
            },
            signer_for(&short.secret_key.secret_bytes())?,
        )
        .await
        .expect_err("one sat short cannot fund the exit");
    assert!(
        matches!(err, SdkError::InsufficientCpfpFunds { required_sat } if required_sat == minimum),
        "got: {err:?} (expected required {minimum})"
    );
    Ok(())
}

/// Two leaves, one UTXO (fan-out). The quote is internally consistent and plans a
/// fan-out. The exact single-UTXO funding minimum is taken from the SDK's own
/// `InsufficientCpfpFunds` report (by funding a floor below it), not from the
/// quote's `single_utxo_funding_sat`: that figure is a close lower-bound estimate
/// sized off one representative input, so it can sit a little under the amount the
/// build actually needs to complete both branches. Funding the reported minimum
/// builds both leaves with a fan-out; one sat less fails.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_two_leaf_fanout_funding_boundary(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;

    // A generous single-UTXO probe: its 2-leaf quote (with a fan-out) and its
    // built set (for the package fees).
    let (quote, probe, generous) = quote_then_build_single(&sdk, CPFP_SATS * 4, FEE_RATE).await?;
    assert_eq!(quote.leaves.len(), 2);
    assert!(quote.fanout_fee_sat > 0);
    assert_quote_consistent(&quote, FEE_RATE, &generous.address.to_string(), p2tr_dust());
    assert!(
        probe
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "a single UTXO across two branches funds a fan-out"
    );
    let dust = generous
        .witness_utxo
        .script_pubkey
        .minimal_non_dust()
        .to_sat();

    // Funding a floor below the fan-out minimum makes the build report the exact
    // requirement.
    let budget_floor = sum_package_fees(&probe, &[&generous])? + 2 * dust;
    let floor = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(budget_floor)).await?;
    let floor_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: floor.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let required = match sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: floor_quote,
                funding_inputs: vec![cpfp_input(&floor)],
            },
            signer_for(&floor.secret_key.secret_bytes())?,
        )
        .await
    {
        Err(SdkError::InsufficientCpfpFunds { required_sat }) => required_sat,
        other => panic!("expected the fan-out requirement at the budget floor, got {other:?}"),
    };

    // Funding exactly the reported requirement builds both leaves with a fan-out.
    let (_, built, _) = quote_then_build_single(&sdk, required, FEE_RATE).await?;
    assert_eq!(built.leaves.len(), 2);
    assert!(
        built
            .transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::FanOut)),
        "the reported minimum funds a two-branch fan-out exit"
    );

    // One sat less fails.
    let short = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(required - 1)).await?;
    let short_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: short.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let err = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: short_quote,
                funding_inputs: vec![cpfp_input(&short)],
            },
            signer_for(&short.secret_key.secret_bytes())?,
        )
        .await
        .expect_err("one sat short of the fan-out requirement");
    assert!(
        matches!(err, SdkError::InsufficientCpfpFunds { .. }),
        "got: {err:?}"
    );
    Ok(())
}

/// A `Custom` funding input (here a P2TR script declared explicitly with its
/// signed input weight) is honored: the exit builds.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_custom_funding_input(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let utxo = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let script_pubkey_hex = hex::encode(utxo.witness_utxo.script_pubkey.as_bytes());

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::Custom {
                script_pubkey_hex: script_pubkey_hex.clone(),
                signed_input_weight: 230,
            },
            destination: utxo.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    assert_quote_consistent(&quote, FEE_RATE, &utxo.address.to_string(), p2tr_dust());

    let custom = CpfpInput::Custom {
        txid: utxo.outpoint.txid.to_string(),
        vout: utxo.outpoint.vout,
        value: utxo.witness_utxo.value.to_sat(),
        script_pubkey_hex,
        signed_input_weight: 230,
    };
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![custom],
            },
            signer_for(&utxo.secret_key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert!(
        resp.transactions
            .iter()
            .any(|t| matches!(t.kind, UnilateralExitTxKind::Sweep)),
        "the custom-funded exit still produces a sweep"
    );
    Ok(())
}

/// All node packages confirmed, refund not: re-preparing shows every node
/// `Confirmed` (no CPFP child), while the refund and sweep stay unconfirmed and
/// the refund's CPFP resumes off the last confirmed node's on-chain change.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_all_nodes_confirmed_resumes_at_refund(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    // Broadcast and confirm every node package (topological order), not the refund.
    let node_txids: Vec<String> = first
        .transactions
        .iter()
        .filter(|t| matches!(t.kind, UnilateralExitTxKind::Node))
        .map(|t| t.txid.clone())
        .collect();
    assert!(!node_txids.is_empty(), "expected at least one node package");
    for entry in &first.transactions {
        if matches!(entry.kind, UnilateralExitTxKind::Node) {
            broadcast_and_mine(&sdk, entry).await?;
        }
    }

    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let second_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: second_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    for txid in &node_txids {
        let node = second
            .transactions
            .iter()
            .find(|t| &t.txid == txid)
            .expect("the confirmed node must still appear");
        assert!(
            matches!(node.status, ConfirmationStatus::Confirmed),
            "node {txid} should be confirmed"
        );
        assert!(
            node.cpfp_tx_hex.is_none(),
            "a confirmed node carries no CPFP child"
        );
    }
    let refund = second
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Refund))
        .expect("a refund entry");
    assert!(
        matches!(refund.status, ConfirmationStatus::Unconfirmed),
        "the refund is still unconfirmed"
    );
    assert!(
        refund.cpfp_tx_hex.is_some(),
        "the unconfirmed refund carries a CPFP child"
    );
    Ok(())
}

/// The SDK accepts declared funding that is not yet confirmed on-chain: a
/// single-leaf exit prepares from an unmined funding UTXO, and a CPFP child
/// spends that outpoint.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_unconfirmed_funding_accepted(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo_unmined(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote.clone(),
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    assert_build_matches_quote(&quote, &resp);
    assert_eq!(resp.leaves.len(), 1);
    let spends_funding = resp
        .transactions
        .iter()
        .filter_map(|t| t.cpfp_tx_hex.as_ref())
        .filter_map(|h| decode_tx(h).ok())
        .any(|tx| tx.input.iter().any(|i| i.previous_output == cpfp.outpoint));
    assert!(
        spends_funding,
        "a CPFP child spends the unconfirmed funding UTXO"
    );
    Ok(())
}

/// Mine a self-fee transaction's relative CSV, then broadcast it on its own (no
/// package). Used to broadcast a pre-signed self-fee refund (the watchtower
/// path), which pays its own fee and so needs no CPFP child.
async fn mine_csv_then_broadcast(sdk: &LocalSdk, tx: &Transaction) -> Result<()> {
    let csv = tx
        .input
        .iter()
        .filter_map(|i| match i.sequence.to_relative_lock_time()? {
            bitcoin::relative::LockTime::Blocks(h) => Some(u32::from(h.value())),
            bitcoin::relative::LockTime::Time(_) => None,
        })
        .max()
        .unwrap_or(0);
    if csv > 0 {
        sdk.fixtures.bitcoind.generate_blocks(csv.into()).await?;
    }
    sdk.fixtures
        .bitcoind
        .broadcast_transaction_no_fee_check(tx)
        .await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;
    Ok(())
}

/// A leaf's self-fee `direct_from_cpfp_refund_tx` is a valid exit path to the
/// user's key that needs no CPFP child. After the cpfp node chain confirms, this
/// refund broadcasts on its own; the exit's sweep then recovers it to the
/// destination, recognized by the leaf's refund address (the same address every
/// refund variant pays). This exercises the address-based recovery against a
/// non-cpfp refund actually on-chain.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_sweep_recovers_direct_from_cpfp_refund(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;

    // Capture the leaf's pre-signed self-fee refund up front (it spends the
    // leaf's own cpfp node_tx output and pays the user's key).
    let leaf = sdk
        .spark_wallet
        .list_leaves()
        .await?
        .available
        .into_iter()
        .next()
        .expect("a claimed leaf");
    let leaf_id = leaf.id.clone();
    let refund = leaf
        .direct_from_cpfp_refund_tx
        .clone()
        .expect("a claimed leaf carries a direct_from_cpfp refund");
    assert_eq!(
        refund.input[0].previous_output.txid,
        leaf.node_tx.compute_txid(),
        "the direct_from_cpfp refund must spend the leaf's node_tx"
    );

    // Drive the cpfp chain so the leaf's node_tx output is on-chain, then
    // broadcast the self-fee refund instead of the cpfp refund.
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let resp = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;
    for entry in &resp.transactions {
        if matches!(entry.kind, UnilateralExitTxKind::Node) {
            broadcast_and_mine(&sdk, entry).await?;
        }
    }
    mine_csv_then_broadcast(&sdk, &refund).await?;

    // The sweep recovers that on-chain refund to the destination.
    let dest = cpfp.address.clone();
    let refund_output = RefundOutput {
        outpoint: OutPoint {
            txid: refund.compute_txid(),
            vout: 0,
        },
        leaf_id,
        value: refund.output[0].value.to_sat(),
    };
    let sweep_psbt = sdk
        .spark_wallet
        .create_refund_sweep_transaction(vec![refund_output], vec![], dest.clone(), FEE_RATE_KW)
        .await?;
    let sweep = sweep_psbt.extract_tx_unchecked_fee_rate();
    let sweep_txid = sdk.fixtures.bitcoind.broadcast_transaction(&sweep).await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;

    let confirmed = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    assert_eq!(confirmed.compute_txid(), sweep_txid);
    assert!(
        confirmed
            .output
            .iter()
            .any(|o| o.script_pubkey == dest.script_pubkey()),
        "the sweep pays the destination"
    );
    Ok(())
}

// ===========================================================================
// Foreign-CPFP resume: a tree already partially confirmed by a THIRD PARTY's
// fee-bump (a CPFP child whose fee comes from a UTXO that is not the exit's
// funding, and whose change pays a script the exit does not recognize) must
// still resume, funding the remaining frontier from our own supplied inputs.
//
// TREE SHAPE: a claimed deposit is a single-node tree — `create_tree_root`
// (spark `services/deposit.rs`) creates exactly one node that is simultaneously
// the ROOT and the LEAF, plus that leaf's refund. So:
//   - ROOT and LEAF "confirmed-foreign" coincide: the single node is both, and
//     `test_node_confirmed_by_foreign_cpfp_resumes` covers both at once.
//   - There is NO INTERMEDIATE node to confirm, so the intermediate level is
//     unreachable with this harness and is intentionally not tested (it would
//     need a multi-level tree via a tree-split / SSP op the local fixture can't
//     produce — the same limitation the file already notes for shared-ancestor
//     multi-leaf trees). The intermediate "not ours" path is unit-covered in
//     spark-wallet's `exit_build_tests`.
//   - The REFUND level is covered by
//     `test_refund_confirmed_by_foreign_cpfp_is_adopted`.

/// A CPFP child that confirms `parent_tx` but whose change pays `foreign`'s own
/// key — a script the exit never funds, so on resume the confirming change is not
/// recognizable as ours. v3/TRUC, anchor spent last, flat `fee_sat`.
fn build_foreign_cpfp_child(
    parent_tx: &Transaction,
    foreign: &FundedUtxo,
    fee_sat: u64,
) -> Result<Transaction> {
    let (anchor_vout, anchor_out) = parent_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, o)| is_ephemeral_anchor_output(o))
        .expect("parent carries an ephemeral anchor to bump");
    let rbf = Sequence(0xffff_fffd);
    let change_value = foreign
        .witness_utxo
        .value
        .to_sat()
        .checked_sub(fee_sat)
        .expect("the foreign UTXO covers the CPFP fee");
    let unsigned = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![
            TxIn {
                previous_output: foreign.outpoint,
                sequence: rbf,
                ..Default::default()
            },
            TxIn {
                previous_output: OutPoint {
                    txid: parent_tx.compute_txid(),
                    vout: u32::try_from(anchor_vout)?,
                },
                sequence: rbf,
                ..Default::default()
            },
        ],
        output: vec![TxOut {
            value: Amount::from_sat(change_value),
            script_pubkey: foreign.address.script_pubkey(),
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(unsigned)?;
    psbt.inputs[0].witness_utxo = Some(foreign.witness_utxo.clone());
    psbt.inputs[1].witness_utxo = Some(anchor_out.clone());
    sign_cpfp_psbt_p2tr(&psbt, &foreign.secret_key)
}

/// Like [`assert_fee_rate`] (`near_exact = false`) but for a RESUMED, partially
/// confirmed set: `Confirmed` node/refund entries carry no CPFP child, so they are
/// skipped rather than unwrapped. Every still-driven package and the sweep must
/// still pay at least the target rate for its actual weight.
fn assert_unconfirmed_fee_rate(
    resp: &UnilateralExitResponse,
    external: &[&FundedUtxo],
    rate: u64,
) -> Result<()> {
    let map = output_value_map(resp, external)?;
    let target = |weight: u64| weight.saturating_mul(rate).div_ceil(1000);
    for entry in &resp.transactions {
        let (fee, weight) = match entry.kind {
            UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund => {
                // A confirmed (adopted / already-on-chain) step has no child to bump.
                let Some(child_hex) = entry.cpfp_tx_hex.as_ref() else {
                    continue;
                };
                let parent = decode_tx(&entry.tx_hex)?;
                let child = decode_tx(child_hex)?;
                let child_in = tx_input_value(&child, &map).expect("child input values known");
                let child_out: u64 = child.output.iter().map(|o| o.value.to_sat()).sum();
                (
                    child_in - child_out,
                    parent.weight().to_wu() + child.weight().to_wu(),
                )
            }
            UnilateralExitTxKind::FanOut | UnilateralExitTxKind::Sweep => {
                let tx = decode_tx(&entry.tx_hex)?;
                let tx_in = tx_input_value(&tx, &map).expect("input values known");
                let tx_out: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
                (tx_in - tx_out, tx.weight().to_wu())
            }
        };
        let t = target(weight);
        assert!(
            fee >= t,
            "{:?} fee {fee} is below the target rate ({t} for weight {weight}, rate {rate})",
            entry.kind
        );
    }
    Ok(())
}

/// The confirmed-resume counterpart of [`assert_all_mined`]: broadcast and mine a
/// partially confirmed exit. Already-`Confirmed` node/refund/fan-out entries are
/// on-chain, so they are skipped rather than re-broadcast; every unconfirmed entry
/// is driven, and the sweep is broadcast last. Then assert every entry reads back
/// from a block and the sweep pays `destination`.
async fn assert_resumed_all_mined(
    sdk: &LocalSdk,
    built: &UnilateralExitResponse,
    destination: &Address,
) -> Result<()> {
    let mut sweep_txid: Option<Txid> = None;
    for entry in &built.transactions {
        match entry.kind {
            UnilateralExitTxKind::Node | UnilateralExitTxKind::Refund => {
                if matches!(entry.status, ConfirmationStatus::Confirmed) {
                    continue;
                }
                broadcast_and_mine(sdk, entry).await?;
            }
            UnilateralExitTxKind::FanOut => {
                if matches!(entry.status, ConfirmationStatus::Confirmed) {
                    continue;
                }
                let tx = decode_tx(&entry.tx_hex)?;
                sdk.fixtures.bitcoind.broadcast_transaction(&tx).await?;
                sdk.fixtures.bitcoind.generate_blocks(1).await?;
            }
            UnilateralExitTxKind::Sweep => {
                let tx = decode_tx(&entry.tx_hex)?;
                let txid = sdk.fixtures.bitcoind.broadcast_transaction(&tx).await?;
                sdk.fixtures.bitcoind.generate_blocks(1).await?;
                sweep_txid = Some(txid);
            }
        }
    }

    for entry in &built.transactions {
        let txid = Txid::from_str(&entry.txid)?;
        let mined = sdk.fixtures.bitcoind.get_transaction(&txid).await?;
        assert_eq!(
            mined.compute_txid(),
            txid,
            "{:?} {} was not mined",
            entry.kind,
            entry.txid
        );
    }

    let sweep_txid = sweep_txid.expect("the resumed set terminates in a sweep");
    let sweep = sdk.fixtures.bitcoind.get_transaction(&sweep_txid).await?;
    assert!(
        sweep
            .output
            .iter()
            .any(|o| o.script_pubkey == destination.script_pubkey()),
        "the resumed sweep must pay the destination"
    );
    Ok(())
}

/// A tree node confirmed on-chain by a FOREIGN CPFP (change paying a third-party
/// script) still resumes: the node comes back `Confirmed` with no child, and the
/// frontier below it (the leaf's refund) is driven fresh from our own funding. A
/// claimed deposit is a single-node tree, so this covers the root and leaf levels
/// of the "not ours" path.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_node_confirmed_by_foreign_cpfp_resumes(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let leaf_ids: Vec<String> = first_quote
        .leaves
        .iter()
        .map(|l| l.leaf_id.clone())
        .collect();
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    let node_pkg = first
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Node))
        .expect("a node package");
    let node_txid = node_pkg.txid.clone();
    let node_tx = decode_tx(&node_pkg.tx_hex)?;
    let foreign = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let foreign_child = build_foreign_cpfp_child(&node_tx, &foreign, 5_000)?;
    submit_package_with_csv_retry(&sdk.fixtures.bitcoind, &node_tx, &foreign_child).await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;

    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let destination = cpfp.address.clone();
    let second_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.to_string(),
            selection: ExitLeafSelection::Specific {
                leaf_ids: leaf_ids.clone(),
            },
        })
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: second_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    let resumed_node = second
        .transactions
        .iter()
        .find(|t| t.txid == node_txid)
        .expect("the foreign-confirmed node must still appear");
    assert!(
        matches!(resumed_node.status, ConfirmationStatus::Confirmed),
        "the node confirmed by a foreign CPFP resumes as Confirmed"
    );
    assert!(
        resumed_node.cpfp_tx_hex.is_none(),
        "a confirmed node carries no CPFP child, even when a third party confirmed it"
    );

    let refund = second
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Refund))
        .expect("a refund entry");
    assert!(
        matches!(refund.status, ConfirmationStatus::Unconfirmed),
        "the refund below the confirmed node resumes unconfirmed"
    );
    let refund_child = decode_tx(
        refund
            .cpfp_tx_hex
            .as_ref()
            .expect("the driven refund carries a fresh CPFP child"),
    )?;
    assert!(
        refund_child
            .input
            .iter()
            .any(|i| i.previous_output == cpfp.outpoint),
        "the refund's CPFP child funds the frontier from our freshly-supplied UTXO"
    );

    assert_unconfirmed_fee_rate(&second, &[&cpfp], FEE_RATE_KW)?;
    assert_resumed_all_mined(&sdk, &second, &destination).await?;
    Ok(())
}

/// A leaf's refund confirmed on-chain by a FOREIGN CPFP is adopted on resume via
/// the refund-address scan (recognition is address-based, independent of who paid
/// the fee): it comes back `Confirmed` with the same txid and no rebuilt child,
/// and the resumed exit sweeps it. The refund level of the foreign-confirmation
/// coverage.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn test_refund_confirmed_by_foreign_cpfp_is_adopted(
    #[case] backend: SignerBackend,
) -> Result<()> {
    let sdk = new_local_sdk(backend).await?;
    deposit_and_claim(&sdk, Amount::from_sat(LEAF_SATS)).await?;
    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;

    let first_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: cpfp.address.to_string(),
            selection: ExitLeafSelection::Auto,
        })
        .await?;
    let leaf_ids: Vec<String> = first_quote
        .leaves
        .iter()
        .map(|l| l.leaf_id.clone())
        .collect();
    let first = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: first_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    let node_pkg = first
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Node))
        .expect("a node package")
        .clone();
    broadcast_and_mine(&sdk, &node_pkg).await?;

    let refund_pkg = first
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Refund))
        .expect("a refund package");
    let refund_txid = refund_pkg.txid.clone();
    let refund_tx = decode_tx(&refund_pkg.tx_hex)?;
    let foreign = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let foreign_child = build_foreign_cpfp_child(&refund_tx, &foreign, 5_000)?;
    submit_package_with_csv_retry(&sdk.fixtures.bitcoind, &refund_tx, &foreign_child).await?;
    sdk.fixtures.bitcoind.generate_blocks(1).await?;

    let cpfp = fund_p2tr_utxo(&sdk.fixtures.bitcoind, Amount::from_sat(CPFP_SATS)).await?;
    let destination = cpfp.address.clone();
    let second_quote = sdk
        .sdk
        .prepare_unilateral_exit(PrepareUnilateralExitRequest {
            fee_rate_sat_per_vbyte: FEE_RATE,
            funding_kind: CpfpFundingKind::P2tr,
            destination: destination.to_string(),
            selection: ExitLeafSelection::Specific {
                leaf_ids: leaf_ids.clone(),
            },
        })
        .await?;
    let second = sdk
        .sdk
        .unilateral_exit(
            UnilateralExitRequest {
                prepared: second_quote,
                funding_inputs: vec![cpfp_input(&cpfp)],
            },
            signer_for(&cpfp.secret_key.secret_bytes())?,
        )
        .await?;

    let resumed_refund = second
        .transactions
        .iter()
        .find(|t| matches!(t.kind, UnilateralExitTxKind::Refund))
        .expect("a refund entry");
    assert!(
        matches!(resumed_refund.status, ConfirmationStatus::Confirmed),
        "the foreign-confirmed refund resumes as Confirmed"
    );
    assert_eq!(
        resumed_refund.txid, refund_txid,
        "adopts the on-chain refund, not a freshly built one"
    );
    assert!(
        resumed_refund.cpfp_tx_hex.is_none(),
        "an adopted refund carries no rebuilt CPFP child"
    );

    assert_unconfirmed_fee_rate(&second, &[&cpfp], FEE_RATE_KW)?;
    assert_resumed_all_mined(&sdk, &second, &destination).await?;
    Ok(())
}
