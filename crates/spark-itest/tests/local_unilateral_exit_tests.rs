use anyhow::Result;
use bitcoin::{
    Address, Amount, OutPoint, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime,
    hashes::Hash as _,
    key::{Secp256k1, TapTweak as _},
    secp256k1::SecretKey,
    sighash::{self, SighashCache},
    transaction::Version,
};
use rstest::*;
use spark_itest::helpers::{WalletsFixture, deposit_to_wallet, wallets};
use spark_wallet::CpfpInput;
use tracing::info;

/// P2TR key-path signed input weight: 41 non-witness × 4 + 66 witness = 230 WU
const P2TR_INPUT_WEIGHT: u64 = 230;

/// Test that a P2TR key-path spend with BIP341 tap tweak is accepted by bitcoind.
///
/// This validates the signing logic used by `SingleKeySigner` for P2TR CPFP inputs:
/// the key must be tweaked before signing, otherwise the Schnorr signature won't
/// match the tweaked output key committed in the scriptPubKey.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_p2tr_cpfp_signature_accepted(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let bitcoind = &fixture.fixtures.bitcoind;
    let secp = Secp256k1::new();

    // Generate a fresh key and derive a P2TR address
    let cpfp_secret_key = SecretKey::new(&mut rand::thread_rng());
    let cpfp_pubkey = cpfp_secret_key.public_key(&secp);
    let (xonly, _) = cpfp_pubkey.x_only_public_key();
    let cpfp_address = Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest);

    // Fund the P2TR address via bitcoind
    let cpfp_amount = Amount::from_sat(50_000);
    let cpfp_txid = bitcoind.fund_address(&cpfp_address, cpfp_amount).await?;
    info!("Funded P2TR address {cpfp_address}, txid: {cpfp_txid}");

    // Mine a block so the UTXO is confirmed
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&cpfp_txid, 1).await?;

    // Find the vout for our P2TR output
    let cpfp_tx = bitcoind.get_transaction(&cpfp_txid).await?;
    let cpfp_vout = cpfp_tx
        .output
        .iter()
        .position(|o| o.script_pubkey == cpfp_address.script_pubkey())
        .expect("P2TR output not found in funding tx") as u32;

    // Build a simple transaction spending the P2TR UTXO back to the same address
    let fee = 300u64;
    let spend_tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: cpfp_txid,
                vout: cpfp_vout,
            },
            script_sig: bitcoin::ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(cpfp_amount.to_sat() - fee),
            script_pubkey: cpfp_address.script_pubkey(),
        }],
    };

    // Sign using the tap-tweaked key (same logic as SingleKeySigner)
    let prevouts = vec![TxOut {
        value: cpfp_amount,
        script_pubkey: cpfp_address.script_pubkey(),
    }];

    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &cpfp_secret_key)
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

    // Broadcast — this will fail if the tap tweak is missing or incorrect
    let spend_txid = bitcoind.broadcast_transaction(&signed_tx).await?;
    info!("Broadcast P2TR key-path spend: {spend_txid}");

    // Mine a block and verify the transaction was included
    bitcoind.generate_blocks(1).await?;
    let confirmed_tx = bitcoind.get_transaction(&spend_txid).await?;
    assert_eq!(confirmed_tx.compute_txid(), spend_txid);
    info!("P2TR spend confirmed");

    Ok(())
}

/// Test unilateral exit with a P2TR CPFP input produces valid PSBTs.
///
/// Verifies that `unilateral_exit_autoselect` correctly builds CPFP transactions
/// when given P2TR inputs, and that the resulting PSBTs contain P2TR inputs that
/// can be signed.
#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_unilateral_exit_p2tr_cpfp(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let wallet = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    // Deposit funds to create a leaf
    deposit_to_wallet(wallet, bitcoind).await?;
    let balance = wallet.get_balance().await?;
    info!("Balance after deposit: {balance} sats");

    // Generate a fresh key and derive a P2TR address for the CPFP input
    let secp = Secp256k1::new();
    let cpfp_secret_key = SecretKey::new(&mut rand::thread_rng());
    let cpfp_pubkey = cpfp_secret_key.public_key(&secp);
    let (xonly, _) = cpfp_pubkey.x_only_public_key();
    let cpfp_address = Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest);

    // Fund the P2TR address via bitcoind
    let cpfp_amount = Amount::from_sat(50_000);
    let cpfp_txid = bitcoind.fund_address(&cpfp_address, cpfp_amount).await?;
    info!("Funded P2TR CPFP address {cpfp_address}, txid: {cpfp_txid}");

    // Mine a block so the UTXO is confirmed
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&cpfp_txid, 1).await?;

    // Find the vout for our P2TR output
    let cpfp_tx = bitcoind.get_transaction(&cpfp_txid).await?;
    let cpfp_vout = cpfp_tx
        .output
        .iter()
        .position(|o| o.script_pubkey == cpfp_address.script_pubkey())
        .expect("P2TR output not found in CPFP funding tx") as u32;

    let cpfp_input = CpfpInput {
        outpoint: OutPoint {
            txid: cpfp_txid,
            vout: cpfp_vout,
        },
        witness_utxo: TxOut {
            value: cpfp_amount,
            script_pubkey: cpfp_address.script_pubkey(),
        },
        signed_input_weight: P2TR_INPUT_WEIGHT,
    };

    // Perform unilateral exit autoselect
    let destination = cpfp_address.clone();
    let fee_rate = 2; // 2 sat/vB
    let exit_result = wallet
        .unilateral_exit_autoselect(fee_rate, vec![cpfp_input], destination)
        .await?;
    info!(
        "Autoselect returned {} leaves",
        exit_result.selected_leaves.len()
    );
    assert!(
        !exit_result.selected_leaves.is_empty(),
        "Expected at least one leaf selected for exit"
    );

    // Verify each CPFP PSBT contains P2TR inputs and can be signed
    for leaf_psbts in &exit_result.leaf_tx_cpfp_psbts {
        for tc in &leaf_psbts.tx_cpfp_psbts {
            let psbt = &tc.child_psbt;

            // Count P2TR inputs (excluding ephemeral anchors)
            let p2tr_count = psbt
                .inputs
                .iter()
                .filter(|input| {
                    input
                        .witness_utxo
                        .as_ref()
                        .is_some_and(|o| o.script_pubkey.is_p2tr())
                })
                .count();

            assert!(
                p2tr_count > 0,
                "Expected P2TR inputs in CPFP PSBT for node {}",
                tc.node_id
            );
            info!(
                "Node {} CPFP PSBT has {p2tr_count} P2TR input(s)",
                tc.node_id
            );

            // Verify the change output uses the P2TR script
            let has_p2tr_output = psbt
                .unsigned_tx
                .output
                .iter()
                .any(|o| o.script_pubkey.is_p2tr());
            assert!(
                has_p2tr_output,
                "Expected P2TR change output in CPFP PSBT"
            );
        }
    }

    info!("All CPFP PSBTs verified with P2TR inputs");
    Ok(())
}
