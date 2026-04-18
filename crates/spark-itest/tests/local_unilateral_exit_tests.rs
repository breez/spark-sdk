use std::collections::HashSet;

use anyhow::{Result, bail};
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, Psbt, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
    absolute::LockTime,
    ecdsa::Signature as EcdsaSignature,
    hashes::Hash as _,
    key::{Secp256k1, TapTweak as _},
    secp256k1::SecretKey,
    sighash::{self, SighashCache},
    transaction::Version,
};
use rstest::*;
use spark_itest::{
    fixtures::bitcoind::BitcoindFixture,
    helpers::{WalletsFixture, deposit_to_wallet, deposit_with_amount, wallets},
};
use spark_wallet::CpfpInput;
use tracing::info;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// P2TR key-path signed input weight: 41 non-witness × 4 + 66 witness = 230 WU
const P2TR_INPUT_WEIGHT: u64 = 230;
/// P2WPKH signed input weight: 41 non-witness × 4 + 108 witness = 272 WU
const P2WPKH_INPUT_WEIGHT: u64 = 272;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Funded UTXO ready for use as a CPFP input.
struct FundedUtxo {
    secret_key: SecretKey,
    outpoint: OutPoint,
    witness_utxo: TxOut,
    address: Address,
}

/// Fund a new P2TR address from bitcoind and return the UTXO details.
async fn fund_p2tr_utxo(bitcoind: &BitcoindFixture, amount: Amount) -> Result<FundedUtxo> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::new(&mut rand::thread_rng());
    let pubkey = secret_key.public_key(&secp);
    let (xonly, _) = pubkey.x_only_public_key();
    let address = Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest);

    let txid = bitcoind.fund_address(&address, amount).await?;
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;

    let tx = bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .position(|o| o.script_pubkey == address.script_pubkey())
        .expect("P2TR output not found") as u32;

    Ok(FundedUtxo {
        secret_key,
        outpoint: OutPoint { txid, vout },
        witness_utxo: TxOut {
            value: amount,
            script_pubkey: address.script_pubkey(),
        },
        address,
    })
}

/// Fund a new P2WPKH address from bitcoind and return the UTXO details.
async fn fund_p2wpkh_utxo(bitcoind: &BitcoindFixture, amount: Amount) -> Result<FundedUtxo> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::new(&mut rand::thread_rng());
    let pubkey = secret_key.public_key(&secp);
    let compressed = CompressedPublicKey(pubkey);
    let address = Address::p2wpkh(&compressed, bitcoin::Network::Regtest);

    let txid = bitcoind.fund_address(&address, amount).await?;
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;

    let tx = bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .position(|o| o.script_pubkey == address.script_pubkey())
        .expect("P2WPKH output not found") as u32;

    Ok(FundedUtxo {
        secret_key,
        outpoint: OutPoint { txid, vout },
        witness_utxo: TxOut {
            value: amount,
            script_pubkey: address.script_pubkey(),
        },
        address,
    })
}

fn make_cpfp_input(utxo: &FundedUtxo, weight: u64) -> CpfpInput {
    CpfpInput {
        outpoint: utxo.outpoint,
        witness_utxo: utxo.witness_utxo.clone(),
        signed_input_weight: weight,
    }
}

/// Sign a CPFP PSBT that has P2TR external inputs. Finalizes anchor + P2TR inputs.
fn sign_cpfp_psbt_p2tr(psbt: &Psbt, secret_key: &SecretKey) -> Result<Transaction> {
    let mut psbt = psbt.clone();
    finalize_anchor_inputs(&mut psbt);

    let secp = Secp256k1::new();
    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, secret_key)
        .tap_tweak(&secp, None)
        .to_keypair();

    let prevouts: Vec<TxOut> = psbt
        .inputs
        .iter()
        .map(|i| i.witness_utxo.clone().unwrap_or(TxOut::NULL))
        .collect();
    let prevouts_ref = sighash::Prevouts::All(&prevouts);

    let taproot_indices: Vec<usize> = psbt
        .inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            i.final_script_witness.is_none()
                && i.witness_utxo
                    .as_ref()
                    .is_some_and(|o| o.script_pubkey.is_p2tr())
        })
        .map(|(idx, _)| idx)
        .collect();

    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    for i in taproot_indices {
        let sighash = cache.taproot_key_spend_signature_hash(
            i,
            &prevouts_ref,
            sighash::TapSighashType::Default,
        )?;
        let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
        let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        let tap_sig = bitcoin::taproot::Signature {
            signature: schnorr_sig,
            sighash_type: sighash::TapSighashType::Default,
        };
        let mut witness = Witness::new();
        witness.push(tap_sig.to_vec());
        psbt.inputs[i].final_script_witness = Some(witness);
    }

    Ok(psbt.extract_tx_unchecked_fee_rate())
}

/// Sign a CPFP PSBT that has P2WPKH external inputs. Finalizes anchor + P2WPKH inputs.
fn sign_cpfp_psbt_p2wpkh(psbt: &Psbt, secret_key: &SecretKey) -> Result<Transaction> {
    let mut psbt = psbt.clone();
    finalize_anchor_inputs(&mut psbt);

    let secp = Secp256k1::new();
    let pubkey = secret_key.public_key(&secp);
    let bitcoin_pubkey = bitcoin::PublicKey::new(pubkey);

    let wpkh_indices: Vec<usize> = psbt
        .inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            i.final_script_witness.is_none()
                && i.witness_utxo
                    .as_ref()
                    .is_some_and(|o| o.script_pubkey.is_p2wpkh())
        })
        .map(|(idx, _)| idx)
        .collect();

    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    for i in wpkh_indices {
        let (msg, ecdsa_type) = psbt
            .sighash_ecdsa(i, &mut cache)
            .map_err(|e| anyhow::anyhow!("ECDSA sighash error: {e}"))?;
        let sig = secp.sign_ecdsa(&msg, secret_key);
        let signature = EcdsaSignature {
            signature: sig,
            sighash_type: ecdsa_type,
        };
        let mut witness = Witness::new();
        witness.push(signature.to_vec());
        witness.push(bitcoin_pubkey.to_bytes());
        psbt.inputs[i].final_script_witness = Some(witness);
    }

    Ok(psbt.extract_tx_unchecked_fee_rate())
}

/// Sign a CPFP PSBT using a caller-provided closure (custom signer).
fn sign_cpfp_psbt_custom<F>(psbt: &Psbt, signer: F) -> Result<Transaction>
where
    F: FnOnce(&mut Psbt) -> Result<()>,
{
    let mut psbt = psbt.clone();
    finalize_anchor_inputs(&mut psbt);
    signer(&mut psbt)?;
    Ok(psbt.extract_tx_unchecked_fee_rate())
}

/// Finalize all ephemeral anchor inputs in a PSBT with an empty witness.
fn finalize_anchor_inputs(psbt: &mut Psbt) {
    for input in &mut psbt.inputs {
        if let Some(ref tx_out) = input.witness_utxo
            && tx_out.value.to_sat() == 0
            && tx_out.script_pubkey.as_bytes() == [0x51, 0x02, 0x4e, 0x73]
        {
            input.final_script_witness = Some(Witness::new());
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone Signature Tests
// ---------------------------------------------------------------------------

/// Test that a P2TR key-path spend with BIP341 tap tweak is accepted by bitcoind.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_p2tr_cpfp_signature_accepted(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let bitcoind = &fixture.fixtures.bitcoind;

    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(50_000)).await?;
    info!(
        "Funded P2TR address {}, txid: {}",
        utxo.address, utxo.outpoint.txid
    );

    let secp = Secp256k1::new();
    let fee = 300u64;
    let spend_tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: utxo.outpoint,
            script_sig: bitcoin::ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_000 - fee),
            script_pubkey: utxo.address.script_pubkey(),
        }],
    };

    let prevouts = vec![utxo.witness_utxo.clone()];
    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &utxo.secret_key)
        .tap_tweak(&secp, None)
        .to_keypair();
    let mut cache = SighashCache::new(&spend_tx);
    let sighash = cache.taproot_key_spend_signature_hash(
        0,
        &sighash::Prevouts::All(&prevouts),
        sighash::TapSighashType::Default,
    )?;
    let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
    let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
    let tap_sig = bitcoin::taproot::Signature {
        signature: schnorr_sig,
        sighash_type: sighash::TapSighashType::Default,
    };

    let mut signed_tx = spend_tx;
    let mut witness = Witness::new();
    witness.push(tap_sig.to_vec());
    signed_tx.input[0].witness = witness;

    let spend_txid = bitcoind.broadcast_transaction(&signed_tx).await?;
    info!("Broadcast P2TR key-path spend: {spend_txid}");

    bitcoind.generate_blocks(1).await?;
    let confirmed_tx = bitcoind.get_transaction(&spend_txid).await?;
    assert_eq!(confirmed_tx.compute_txid(), spend_txid);
    info!("P2TR spend confirmed");

    Ok(())
}

/// Test that a P2WPKH ECDSA spend is accepted by bitcoind.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_p2wpkh_cpfp_signature_accepted(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let bitcoind = &fixture.fixtures.bitcoind;

    let utxo = fund_p2wpkh_utxo(bitcoind, Amount::from_sat(50_000)).await?;
    info!(
        "Funded P2WPKH address {}, txid: {}",
        utxo.address, utxo.outpoint.txid
    );

    let secp = Secp256k1::new();
    let fee = 300u64;
    let spend_tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: utxo.outpoint,
            script_sig: bitcoin::ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_000 - fee),
            script_pubkey: utxo.address.script_pubkey(),
        }],
    };

    let pubkey = utxo.secret_key.public_key(&secp);
    let bitcoin_pubkey = bitcoin::PublicKey::new(pubkey);

    let mut psbt = Psbt::from_unsigned_tx(spend_tx)?;
    psbt.inputs[0].witness_utxo = Some(utxo.witness_utxo.clone());
    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    let (msg, ecdsa_type) = psbt.sighash_ecdsa(0, &mut cache)?;
    let sig = secp.sign_ecdsa(&msg, &utxo.secret_key);
    let signature = EcdsaSignature {
        signature: sig,
        sighash_type: ecdsa_type,
    };

    let mut witness = Witness::new();
    witness.push(signature.to_vec());
    witness.push(bitcoin_pubkey.to_bytes());

    let mut signed_tx = psbt.unsigned_tx.clone();
    signed_tx.input[0].witness = witness;

    let spend_txid = bitcoind.broadcast_transaction(&signed_tx).await?;
    info!("Broadcast P2WPKH spend: {spend_txid}");

    bitcoind.generate_blocks(1).await?;
    let confirmed_tx = bitcoind.get_transaction(&spend_txid).await?;
    assert_eq!(confirmed_tx.compute_txid(), spend_txid);
    info!("P2WPKH spend confirmed");

    Ok(())
}

// ---------------------------------------------------------------------------
// Full Broadcast Tests (using wallet transactions)
// ---------------------------------------------------------------------------

/// Full unilateral exit broadcast with P2TR CPFP inputs.
///
/// Broadcasts the node_tx, waits for CSV timelock, broadcasts refund_tx,
/// then broadcasts the sweep transaction.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_full_exit_broadcast_p2tr(#[future] wallets: WalletsFixture) -> Result<()> {
    full_exit_broadcast_test(wallets.await, InputType::P2tr, 2, true).await
}

/// Full unilateral exit broadcast with P2WPKH CPFP inputs.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_full_exit_broadcast_p2wpkh(#[future] wallets: WalletsFixture) -> Result<()> {
    full_exit_broadcast_test(wallets.await, InputType::P2wpkh, 2, true).await
}

/// Full unilateral exit broadcast with a custom signer closure.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_full_exit_broadcast_custom_signer(#[future] wallets: WalletsFixture) -> Result<()> {
    full_exit_broadcast_test(wallets.await, InputType::Custom, 2, true).await
}

/// Full unilateral exit broadcast where the refund transactions stay in the
/// mempool (unconfirmed) when the sweep is broadcast. Validates the guide's
/// claim that the sweep can follow mempool-only refunds.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_full_exit_broadcast_sweep_on_mempool_refund(
    #[future] wallets: WalletsFixture,
) -> Result<()> {
    full_exit_broadcast_test(wallets.await, InputType::P2tr, 2, false).await
}

enum InputType {
    P2tr,
    P2wpkh,
    Custom,
}

async fn full_exit_broadcast_test(
    fixture: WalletsFixture,
    input_type: InputType,
    fee_rate: u64,
    mine_after_refund_tx: bool,
) -> Result<()> {
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;
    let balance = wallet.get_balance().await?;
    info!("Balance after deposit: {balance} sats");

    let cpfp_amount = Amount::from_sat(100_000);
    let (utxo, weight) = match input_type {
        InputType::P2tr | InputType::Custom => {
            let u = fund_p2tr_utxo(bitcoind, cpfp_amount).await?;
            (u, P2TR_INPUT_WEIGHT)
        }
        InputType::P2wpkh => {
            let u = fund_p2wpkh_utxo(bitcoind, cpfp_amount).await?;
            (u, P2WPKH_INPUT_WEIGHT)
        }
    };
    info!(
        "Funded CPFP address {}, txid: {}",
        utxo.address, utxo.outpoint.txid
    );

    let cpfp_input = make_cpfp_input(&utxo, weight);
    let destination = utxo.address.clone();

    let exit_result = wallet
        .unilateral_exit_autoselect(fee_rate, vec![cpfp_input], destination)
        .await?;
    info!(
        "Autoselect: {} leaves, {} PSBTs per leaf",
        exit_result.selected_leaves.len(),
        exit_result
            .leaf_tx_cpfp_psbts
            .first()
            .map_or(0, |l| l.tx_cpfp_psbts.len())
    );
    assert!(
        !exit_result.selected_leaves.is_empty(),
        "Expected at least one leaf"
    );

    // Sign and broadcast each parent+child package in order
    for leaf_psbts in &exit_result.leaf_tx_cpfp_psbts {
        let tc_count = leaf_psbts.tx_cpfp_psbts.len();
        for (psbt_idx, tc) in leaf_psbts.tx_cpfp_psbts.iter().enumerate() {
            let is_refund_tx = psbt_idx + 1 == tc_count;
            // Log the parent tx's inputs for debugging
            for (i, input) in tc.parent_tx.input.iter().enumerate() {
                info!(
                    "PSBT[{psbt_idx}] parent_tx input[{i}]: {} seq={}",
                    input.previous_output, input.sequence
                );
            }
            let signed_child = match input_type {
                InputType::P2tr => sign_cpfp_psbt_p2tr(&tc.child_psbt, &utxo.secret_key)?,
                InputType::P2wpkh => sign_cpfp_psbt_p2wpkh(&tc.child_psbt, &utxo.secret_key)?,
                InputType::Custom => {
                    let sk = utxo.secret_key;
                    sign_cpfp_psbt_custom(&tc.child_psbt, |psbt| {
                        // Custom signer: manually do P2TR signing with a different code path
                        let secp = Secp256k1::new();
                        let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &sk)
                            .tap_tweak(&secp, None)
                            .to_keypair();
                        let prevouts: Vec<TxOut> = psbt
                            .inputs
                            .iter()
                            .map(|i| i.witness_utxo.clone().unwrap_or(TxOut::NULL))
                            .collect();
                        let prevouts_ref = sighash::Prevouts::All(&prevouts);
                        let mut cache = SighashCache::new(&psbt.unsigned_tx);

                        let indices: Vec<usize> = psbt
                            .inputs
                            .iter()
                            .enumerate()
                            .filter(|(_, i)| {
                                i.final_script_witness.is_none()
                                    && i.witness_utxo
                                        .as_ref()
                                        .is_some_and(|o| o.script_pubkey.is_p2tr())
                            })
                            .map(|(idx, _)| idx)
                            .collect();

                        for i in indices {
                            let sighash = cache.taproot_key_spend_signature_hash(
                                i,
                                &prevouts_ref,
                                sighash::TapSighashType::Default,
                            )?;
                            let msg =
                                bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
                            let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
                            let tap_sig = bitcoin::taproot::Signature {
                                signature: schnorr_sig,
                                sighash_type: sighash::TapSighashType::Default,
                            };
                            let mut witness = Witness::new();
                            witness.push(tap_sig.to_vec());
                            psbt.inputs[i].final_script_witness = Some(witness);
                        }
                        Ok(())
                    })?
                }
            };

            // Submit parent + CPFP child as a package. v3 transactions with
            // ephemeral anchors require package relay.
            let result = bitcoind
                .submit_package(&[&tc.parent_tx, &signed_child])
                .await?;
            let pkg_msg = result
                .get("package_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            info!("Package result for node {}: {pkg_msg}", tc.node_id);

            if pkg_msg != "success" {
                // Check if it's a CSV timelock issue
                let has_bip68_error = result
                    .get("tx-results")
                    .and_then(|v| v.as_object())
                    .is_some_and(|txs| {
                        txs.values().any(|d| {
                            d.get("error")
                                .and_then(|v| v.as_str())
                                .is_some_and(|e| e.contains("non-BIP68-final"))
                        })
                    });
                if has_bip68_error {
                    let csv_blocks = tc
                        .parent_tx
                        .input
                        .first()
                        .map(|i| i.sequence.to_consensus_u32() & 0xFFFF)
                        .unwrap_or(0);
                    info!("CSV timelock: {csv_blocks} blocks. Mining...");
                    bitcoind.generate_blocks(csv_blocks.into()).await?;

                    let retry = bitcoind
                        .submit_package(&[&tc.parent_tx, &signed_child])
                        .await?;
                    let retry_msg = retry
                        .get("package_msg")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    info!("Retry result: {retry_msg}");
                    assert_eq!(
                        retry_msg, "success",
                        "Package still failed after CSV: {retry:?}"
                    );
                } else {
                    bail!(
                        "Package failed: {}",
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    );
                }
            }

            // Mine to confirm, unless the caller wants the refund tx to stay
            // in the mempool so the sweep is broadcast on an unconfirmed parent.
            if !is_refund_tx || mine_after_refund_tx {
                bitcoind.generate_blocks(1).await?;
            }
        }
    }

    // Broadcast the sweep transaction
    let sweep_txid = bitcoind
        .broadcast_transaction(&exit_result.sweep_tx)
        .await?;
    info!("Broadcast sweep tx: {sweep_txid}");

    bitcoind.generate_blocks(1).await?;
    let confirmed_sweep = bitcoind.get_transaction(&sweep_txid).await?;
    assert_eq!(confirmed_sweep.compute_txid(), sweep_txid);
    info!(
        "Sweep tx confirmed. Output value: {} sats",
        confirmed_sweep.output[0].value.to_sat()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// RBF Test
// ---------------------------------------------------------------------------

/// Test that a CPFP child can be replaced via RBF with a higher-fee version.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_cpfp_rbf(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    // Fund two separate CPFP UTXOs
    let utxo_a = fund_p2tr_utxo(bitcoind, Amount::from_sat(100_000)).await?;
    let utxo_b = fund_p2tr_utxo(bitcoind, Amount::from_sat(100_000)).await?;

    // First exit at low fee rate
    let input_a = make_cpfp_input(&utxo_a, P2TR_INPUT_WEIGHT);
    let exit_a = wallet
        .unilateral_exit_autoselect(1, vec![input_a], utxo_a.address.clone())
        .await?;

    // Sign and submit the node_tx package (first PSBT only)
    let first_leaf = &exit_a.leaf_tx_cpfp_psbts[0];
    let first_tc = &first_leaf.tx_cpfp_psbts[0];
    let child_a = sign_cpfp_psbt_p2tr(&first_tc.child_psbt, &utxo_a.secret_key)?;

    let result = bitcoind
        .submit_package(&[&first_tc.parent_tx, &child_a])
        .await?;
    assert_eq!(
        result.get("package_msg").and_then(|v| v.as_str()),
        Some("success"),
        "Initial package should succeed: {result:?}"
    );
    info!("Submitted original CPFP child (fee_rate=1)");

    // Second exit at higher fee rate using different CPFP input
    let input_b = make_cpfp_input(&utxo_b, P2TR_INPUT_WEIGHT);
    let exit_b = wallet
        .unilateral_exit_autoselect(10, vec![input_b], utxo_b.address.clone())
        .await?;

    let first_leaf_b = &exit_b.leaf_tx_cpfp_psbts[0];
    let first_tc_b = &first_leaf_b.tx_cpfp_psbts[0];
    let child_b = sign_cpfp_psbt_p2tr(&first_tc_b.child_psbt, &utxo_b.secret_key)?;

    // The parent_tx (node_tx) is the same, already in mempool.
    // The new child conflicts on the anchor input → RBF replacement.
    let rbf_txid = bitcoind.broadcast_transaction(&child_b).await?;
    info!("RBF replacement accepted: {rbf_txid}");

    // Mine and verify the replacement was confirmed, not the original
    bitcoind.generate_blocks(1).await?;
    let confirmed = bitcoind.get_transaction(&rbf_txid).await?;
    assert_eq!(confirmed.compute_txid(), rbf_txid);
    info!("RBF child confirmed");

    Ok(())
}

// ---------------------------------------------------------------------------
// Wallet-Level PSBT Validation Tests
// ---------------------------------------------------------------------------

/// Test unilateral exit with P2TR CPFP input produces valid PSBTs.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_p2tr_cpfp(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(50_000)).await?;
    let cpfp_input = make_cpfp_input(&utxo, P2TR_INPUT_WEIGHT);

    let exit_result = wallet
        .unilateral_exit_autoselect(2, vec![cpfp_input], utxo.address.clone())
        .await?;
    assert!(!exit_result.selected_leaves.is_empty());

    for leaf_psbts in &exit_result.leaf_tx_cpfp_psbts {
        for tc in &leaf_psbts.tx_cpfp_psbts {
            let p2tr_count = tc
                .child_psbt
                .inputs
                .iter()
                .filter(|i| {
                    i.witness_utxo
                        .as_ref()
                        .is_some_and(|o| o.script_pubkey.is_p2tr())
                })
                .count();
            assert!(p2tr_count > 0, "Expected P2TR inputs in CPFP PSBT");

            let has_p2tr_output = tc
                .child_psbt
                .unsigned_tx
                .output
                .iter()
                .any(|o| o.script_pubkey.is_p2tr());
            assert!(has_p2tr_output, "Expected P2TR change output");
        }
    }

    info!("All CPFP PSBTs verified with P2TR inputs");
    Ok(())
}

/// Test unilateral exit with P2WPKH CPFP input produces valid PSBTs.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_p2wpkh_cpfp(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    let utxo = fund_p2wpkh_utxo(bitcoind, Amount::from_sat(50_000)).await?;
    let cpfp_input = make_cpfp_input(&utxo, P2WPKH_INPUT_WEIGHT);

    let exit_result = wallet
        .unilateral_exit_autoselect(2, vec![cpfp_input], utxo.address.clone())
        .await?;
    assert!(!exit_result.selected_leaves.is_empty());

    for leaf_psbts in &exit_result.leaf_tx_cpfp_psbts {
        for tc in &leaf_psbts.tx_cpfp_psbts {
            let wpkh_count = tc
                .child_psbt
                .inputs
                .iter()
                .filter(|i| {
                    i.witness_utxo
                        .as_ref()
                        .is_some_and(|o| o.script_pubkey.is_p2wpkh())
                })
                .count();
            assert!(wpkh_count > 0, "Expected P2WPKH inputs in CPFP PSBT");

            let has_wpkh_output = tc
                .child_psbt
                .unsigned_tx
                .output
                .iter()
                .any(|o| o.script_pubkey.is_p2wpkh());
            assert!(has_wpkh_output, "Expected P2WPKH change output");
        }
    }

    info!("All CPFP PSBTs verified with P2WPKH inputs");
    Ok(())
}

/// Test that unilateral exit fails when CPFP value is too low for fees + dust.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_insufficient_cpfp_value(
    #[future] wallets: WalletsFixture,
) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    // Fund a tiny CPFP input — too small to cover fees
    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(600)).await?;
    let cpfp_input = make_cpfp_input(&utxo, P2TR_INPUT_WEIGHT);

    let result = wallet
        .unilateral_exit_autoselect(10, vec![cpfp_input], utxo.address.clone())
        .await;

    let err_msg = match result {
        Ok(_) => bail!("Expected error for insufficient CPFP value"),
        Err(e) => e.to_string(),
    };
    info!("Got expected error: {err_msg}");
    assert!(
        err_msg.contains("too low") || err_msg.contains("dust") || err_msg.contains("more sats"),
        "Error should mention budget/dust issue: {err_msg}"
    );

    Ok(())
}

/// Test that unilateral exit at fee_rate=1 (minimum relay fee) broadcasts
/// successfully. This ensures the fee calculation doesn't undershoot — if
/// the package fee is too low, bitcoind will reject it with
/// "min relay fee not met".
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_min_fee_rate(#[future] wallets: WalletsFixture) -> Result<()> {
    full_exit_broadcast_test(wallets.await, InputType::P2tr, 1, true).await
}

/// Test that CPFP PSBTs are correctly chained — each subsequent child
/// spends the previous child's change output.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_cpfp_chain_threading(
    #[future] wallets: WalletsFixture,
) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(100_000)).await?;
    let cpfp_input = make_cpfp_input(&utxo, P2TR_INPUT_WEIGHT);

    let exit_result = wallet
        .unilateral_exit_autoselect(2, vec![cpfp_input.clone()], utxo.address.clone())
        .await?;

    for leaf_psbts in &exit_result.leaf_tx_cpfp_psbts {
        let psbts = &leaf_psbts.tx_cpfp_psbts;

        // First PSBT should reference the original CPFP input
        if let Some(first) = psbts.first() {
            let has_original_input = first
                .child_psbt
                .unsigned_tx
                .input
                .iter()
                .any(|txin| txin.previous_output == cpfp_input.outpoint);
            assert!(
                has_original_input,
                "First CPFP child should spend the original CPFP input"
            );
        }

        // Each subsequent PSBT should spend the previous PSBT's change output
        for window in psbts.windows(2) {
            let prev_child_txid = window[0].child_psbt.unsigned_tx.compute_txid();
            let expected_outpoint = OutPoint {
                txid: prev_child_txid,
                vout: 0,
            };

            let next_has_prev_change = window[1]
                .child_psbt
                .unsigned_tx
                .input
                .iter()
                .any(|txin| txin.previous_output == expected_outpoint);

            assert!(
                next_has_prev_change,
                "PSBT[n+1] should spend PSBT[n]'s change output at {}:0",
                prev_child_txid
            );
        }
    }

    info!("CPFP chain threading verified");
    Ok(())
}

/// Test the sweep transaction structure returned by unilateral_exit_autoselect.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_sweep_tx_structure(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(100_000)).await?;
    let cpfp_input = make_cpfp_input(&utxo, P2TR_INPUT_WEIGHT);

    let exit_result = wallet
        .unilateral_exit_autoselect(2, vec![cpfp_input], utxo.address.clone())
        .await?;

    let sweep_tx = &exit_result.sweep_tx;

    // Sweep tx should be v2 (not v3 — no ephemeral anchor)
    assert_eq!(
        sweep_tx.version,
        Version::TWO,
        "Sweep tx should be version 2"
    );

    // Should have exactly 1 output (the destination)
    assert_eq!(sweep_tx.output.len(), 1, "Sweep tx should have 1 output");

    // Output should go to the destination address
    assert_eq!(
        sweep_tx.output[0].script_pubkey,
        utxo.address.script_pubkey(),
        "Sweep output should go to the destination"
    );

    // Should have one input per selected leaf
    assert_eq!(
        sweep_tx.input.len(),
        exit_result.selected_leaves.len(),
        "Sweep tx should have one input per leaf"
    );

    // Each input should have a non-empty witness (pre-signed by wallet signer)
    for (i, input) in sweep_tx.input.iter().enumerate() {
        assert!(
            !input.witness.is_empty(),
            "Sweep tx input {i} should have a witness"
        );
    }

    // Verify output value: sum of leaf values minus sweep fee
    let total_leaf_value: u64 = exit_result.selected_leaves.iter().map(|l| l.value).sum();
    let output_value = sweep_tx.output[0].value.to_sat();
    assert!(
        output_value < total_leaf_value,
        "Output ({output_value}) should be less than total leaf value ({total_leaf_value}) due to fees"
    );
    assert!(output_value > 0, "Output value should be positive");

    info!(
        "Sweep tx: {} inputs, output {} sats (fee {} sats)",
        sweep_tx.input.len(),
        output_value,
        total_leaf_value - output_value
    );

    Ok(())
}

/// Exits multiple leaves in a single call. Validates that autoselect accepts
/// each leaf, produces one chain per leaf, and builds a sweep that consumes
/// each leaf's refund output.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_multi_leaf(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    // Two separate deposits produce two distinct wallet leaves.
    deposit_with_amount(wallet, bitcoind, 100_000).await?;
    deposit_with_amount(wallet, bitcoind, 100_000).await?;

    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(100_000)).await?;
    let cpfp_input = make_cpfp_input(&utxo, P2TR_INPUT_WEIGHT);

    let exit_result = wallet
        .unilateral_exit_autoselect(2, vec![cpfp_input], utxo.address.clone())
        .await?;

    assert!(
        exit_result.selected_leaves.len() >= 2,
        "expected at least two selected leaves, got {}",
        exit_result.selected_leaves.len()
    );
    assert_eq!(
        exit_result.leaf_tx_cpfp_psbts.len(),
        exit_result.selected_leaves.len(),
        "one CPFP chain per selected leaf"
    );
    assert_eq!(
        exit_result.sweep_tx.input.len(),
        exit_result.selected_leaves.len(),
        "sweep must consume one input per selected leaf"
    );

    // The sweep inputs must point at the last parent_tx of each leaf chain
    // (the refund tx).
    let refund_txids: HashSet<Txid> = exit_result
        .leaf_tx_cpfp_psbts
        .iter()
        .filter_map(|l| l.tx_cpfp_psbts.last())
        .map(|tc| tc.parent_tx.compute_txid())
        .collect();
    let sweep_input_txids: HashSet<Txid> = exit_result
        .sweep_tx
        .input
        .iter()
        .map(|i| i.previous_output.txid)
        .collect();
    assert_eq!(
        refund_txids, sweep_input_txids,
        "sweep inputs must match refund txids"
    );

    Ok(())
}

/// Passing no CPFP inputs to unilateral_exit_autoselect fails with a clear
/// validation error rather than silently producing an invalid exit package.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_empty_inputs(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_to_wallet(wallet, bitcoind).await?;

    let utxo = fund_p2tr_utxo(bitcoind, Amount::from_sat(100_000)).await?;

    let result = wallet
        .unilateral_exit_autoselect(2, vec![], utxo.address.clone())
        .await;

    let err_msg = match result {
        Ok(_) => bail!("Expected an error for empty CPFP inputs"),
        Err(e) => e.to_string(),
    };
    assert!(
        err_msg.to_lowercase().contains("input"),
        "error should mention the missing input: {err_msg}"
    );

    Ok(())
}
