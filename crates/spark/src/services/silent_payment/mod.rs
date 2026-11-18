//! Checks that a cooperative exit transaction pays a BIP-352 silent payment
//! address. The payout key depends on the SSP's private input keys, so the SSP
//! sends the point it shares with the recipient's scan key and a BIP-374 proof
//! that it made that point with the same secret as its input keys.

mod dleq;

use std::{collections::HashSet, fmt};

use bitcoin::{
    CompressedPublicKey, OutPoint, Script, ScriptBuf, Transaction, TxIn, consensus,
    hashes::{Hash, HashEngine, sha256},
    key::TweakedPublicKey,
    secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey, XOnlyPublicKey, constants::GENERATOR_X},
};

use crate::silent_payment::SilentPaymentAddress;

const INPUTS_TAG: &str = "BIP0352/Inputs";
const SHARED_SECRET_TAG: &str = "BIP0352/SharedSecret";

/// The data the SSP returns with a cooperative exit to a silent payment
/// address. Each list has one entry per transaction input, in input order.
#[derive(Clone, Debug, PartialEq)]
pub struct SilentPaymentProof {
    pub output_index: u32,
    pub input_public_keys: Vec<PublicKey>,
    pub input_outpoints: Vec<OutPoint>,
    pub input_prevout_scripts: Vec<ScriptBuf>,
    pub input_prevout_transactions: Vec<Transaction>,
    /// The sum of the SSP's input secrets times the recipient's scan key.
    pub ecdh_share: PublicKey,
    pub dleq_proof: [u8; 64],
}

/// The kind of output an input spends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    P2wpkh,
    P2wsh,
    P2tr,
    OtherWitness,
    P2sh,
    P2pkh,
    Other,
}

impl InputKind {
    fn of(script: &Script) -> Self {
        if script.is_p2wpkh() {
            InputKind::P2wpkh
        } else if script.is_p2wsh() {
            InputKind::P2wsh
        } else if script.is_p2tr() {
            InputKind::P2tr
        } else if script.witness_version().is_some() {
            InputKind::OtherWitness
        } else if script.is_p2sh() {
            InputKind::P2sh
        } else if script.is_p2pkh() {
            InputKind::P2pkh
        } else {
            InputKind::Other
        }
    }
}

impl fmt::Display for InputKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            InputKind::P2wpkh => "P2WPKH",
            InputKind::P2wsh => "P2WSH",
            InputKind::P2tr => "P2TR",
            InputKind::OtherWitness => "segwit",
            InputKind::P2sh => "P2SH",
            InputKind::P2pkh => "P2PKH",
            InputKind::Other => "non-standard",
        };
        f.write_str(name)
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SilentPaymentProofError {
    #[error("the SSP's proof lists {listed} inputs, but the exit transaction has {inputs}")]
    InputCount { listed: usize, inputs: usize },
    #[error("input {0} of the exit transaction is already signed")]
    SignedInput(usize),
    #[error("the exit transaction spends {0} twice")]
    DuplicateOutpoint(OutPoint),
    #[error("the SSP's proof lists {listed} for input {index}, but that input spends {spent}")]
    OutpointMismatch {
        index: usize,
        listed: OutPoint,
        spent: OutPoint,
    },
    #[error("the parent transaction in the SSP's proof for input {0} is not the one it spends")]
    ParentMismatch(usize),
    #[error("input {index} spends output {vout} of a parent transaction without that output")]
    MissingPrevout { index: usize, vout: u32 },
    #[error("the SSP's proof lists a script for input {0} that differs from the one it spends")]
    PrevoutScriptMismatch(usize),
    #[error("input {index} spends a {kind} output, but the SDK accepts only P2WPKH inputs here")]
    UnsupportedInput { index: usize, kind: InputKind },
    #[error("the public key in the SSP's proof for input {0} does not match the output it spends")]
    InputKeyMismatch(usize),
    #[error("the public keys of the exit transaction's inputs add up to no valid key")]
    InputKeySum,
    #[error("the SSP's proof does not show that it used its input keys for the shared point")]
    InvalidDleqProof,
    #[error("the exit transaction's inputs and shared point give no valid output key")]
    InvalidOutputKey,
    #[error("the SSP placed the payout at output {0}, but it has to be output 0")]
    PayoutIndex(u32),
    #[error("output 0 of the exit transaction does not pay the silent payment address")]
    PayoutScript,
    #[error("output 0 of the exit transaction pays {paid} sats, less than the expected {expected}")]
    PayoutAmount { paid: u64, expected: u64 },
}

/// Checks that output 0 of `exit_tx` pays `address` at least
/// `expected_payout_amount_sats`, using the SSP's `proof`.
pub fn validate_silent_payment_payout(
    exit_tx: &Transaction,
    address: &SilentPaymentAddress,
    proof: &SilentPaymentProof,
    expected_payout_amount_sats: u64,
) -> Result<(), SilentPaymentProofError> {
    check_input_lists(exit_tx, proof)?;
    check_distinct_outpoints(exit_tx)?;
    let input_keys = exit_tx
        .input
        .iter()
        .zip(proof_entries(proof))
        .enumerate()
        .map(|(index, (input, entry))| input_key(index, input, &entry))
        .collect::<Result<Vec<_>, _>>()?;
    let input_keys: Vec<&PublicKey> = input_keys.iter().collect();
    let input_key_sum =
        PublicKey::combine_keys(&input_keys).map_err(|_| SilentPaymentProofError::InputKeySum)?;

    if !dleq::verify_proof(
        &input_key_sum,
        &address.scan_key,
        &proof.ecdh_share,
        &proof.dleq_proof,
        &generator(),
        None,
    ) {
        return Err(SilentPaymentProofError::InvalidDleqProof);
    }

    let output_key = input_hash(exit_tx, &input_key_sum)
        .and_then(|input_hash| output_key(&address.spend_key, &proof.ecdh_share, &input_hash))
        .ok_or(SilentPaymentProofError::InvalidOutputKey)?;
    check_payout(
        exit_tx,
        proof.output_index,
        output_key,
        expected_payout_amount_sats,
    )
}

struct ProofEntry<'a> {
    public_key: &'a PublicKey,
    outpoint: &'a OutPoint,
    prevout_script: &'a ScriptBuf,
    prevout_transaction: &'a Transaction,
}

fn proof_entries(proof: &SilentPaymentProof) -> impl Iterator<Item = ProofEntry<'_>> {
    proof
        .input_public_keys
        .iter()
        .zip(&proof.input_outpoints)
        .zip(&proof.input_prevout_scripts)
        .zip(&proof.input_prevout_transactions)
        .map(
            |(((public_key, outpoint), prevout_script), prevout_transaction)| ProofEntry {
                public_key,
                outpoint,
                prevout_script,
                prevout_transaction,
            },
        )
}

fn check_input_lists(
    exit_tx: &Transaction,
    proof: &SilentPaymentProof,
) -> Result<(), SilentPaymentProofError> {
    let inputs = exit_tx.input.len();
    let listed = [
        proof.input_public_keys.len(),
        proof.input_outpoints.len(),
        proof.input_prevout_scripts.len(),
        proof.input_prevout_transactions.len(),
    ];
    match listed.into_iter().find(|listed| *listed != inputs) {
        Some(listed) => Err(SilentPaymentProofError::InputCount { listed, inputs }),
        None => Ok(()),
    }
}

fn check_distinct_outpoints(exit_tx: &Transaction) -> Result<(), SilentPaymentProofError> {
    let mut seen = HashSet::new();
    for input in &exit_tx.input {
        if !seen.insert(input.previous_output) {
            return Err(SilentPaymentProofError::DuplicateOutpoint(
                input.previous_output,
            ));
        }
    }
    Ok(())
}

/// The public key `input` adds to the key sum, checked against the output it
/// spends. A recipient counts a P2WPKH input whatever its witness contains, so the
/// spent output alone settles it. The SDK accepts no other kind.
fn input_key(
    index: usize,
    input: &TxIn,
    entry: &ProofEntry<'_>,
) -> Result<PublicKey, SilentPaymentProofError> {
    if !input.script_sig.is_empty() || !input.witness.is_empty() {
        return Err(SilentPaymentProofError::SignedInput(index));
    }
    if *entry.outpoint != input.previous_output {
        return Err(SilentPaymentProofError::OutpointMismatch {
            index,
            listed: *entry.outpoint,
            spent: input.previous_output,
        });
    }
    if entry.prevout_transaction.compute_txid() != input.previous_output.txid {
        return Err(SilentPaymentProofError::ParentMismatch(index));
    }
    let vout = input.previous_output.vout;
    let prevout = usize::try_from(vout)
        .ok()
        .and_then(|vout| entry.prevout_transaction.output.get(vout))
        .ok_or(SilentPaymentProofError::MissingPrevout { index, vout })?;
    if prevout.script_pubkey != *entry.prevout_script {
        return Err(SilentPaymentProofError::PrevoutScriptMismatch(index));
    }

    match InputKind::of(&prevout.script_pubkey) {
        InputKind::P2wpkh => {}
        kind @ (InputKind::P2wsh
        | InputKind::P2tr
        | InputKind::OtherWitness
        | InputKind::P2sh
        | InputKind::P2pkh
        | InputKind::Other) => {
            return Err(SilentPaymentProofError::UnsupportedInput { index, kind });
        }
    }
    let key_hash = CompressedPublicKey(*entry.public_key).wpubkey_hash();
    if ScriptBuf::new_p2wpkh(&key_hash) != prevout.script_pubkey {
        return Err(SilentPaymentProofError::InputKeyMismatch(index));
    }
    Ok(*entry.public_key)
}

/// BIP-352's `input_hash`, over the smallest outpoint the transaction spends.
fn input_hash(exit_tx: &Transaction, input_key_sum: &PublicKey) -> Option<SecretKey> {
    let smallest_outpoint = exit_tx
        .input
        .iter()
        .map(|input| consensus::serialize(&input.previous_output))
        .min()?;
    let hash = tagged_hash(
        INPUTS_TAG,
        &[&smallest_outpoint, &input_key_sum.serialize()],
    );
    SecretKey::from_slice(&hash).ok()
}

/// The taproot output key of the first output to a silent payment address.
fn output_key(
    spend_key: &PublicKey,
    ecdh_share: &PublicKey,
    input_hash: &SecretKey,
) -> Option<XOnlyPublicKey> {
    let secp = Secp256k1::verification_only();
    let shared_secret = ecdh_share
        .mul_tweak(&secp, &Scalar::from(*input_hash))
        .ok()?;
    let tweak = tagged_hash(
        SHARED_SECRET_TAG,
        &[&shared_secret.serialize(), &0u32.to_be_bytes()],
    );
    let tweak = SecretKey::from_slice(&tweak).ok()?;
    let output_key = spend_key.add_exp_tweak(&secp, &Scalar::from(tweak)).ok()?;
    Some(output_key.x_only_public_key().0)
}

fn check_payout(
    exit_tx: &Transaction,
    output_index: u32,
    output_key: XOnlyPublicKey,
    expected_payout_amount_sats: u64,
) -> Result<(), SilentPaymentProofError> {
    if output_index != 0 {
        return Err(SilentPaymentProofError::PayoutIndex(output_index));
    }
    let payout = exit_tx
        .output
        .first()
        .ok_or(SilentPaymentProofError::PayoutScript)?;
    let expected_script =
        ScriptBuf::new_p2tr_tweaked(TweakedPublicKey::dangerous_assume_tweaked(output_key));
    if payout.script_pubkey != expected_script {
        return Err(SilentPaymentProofError::PayoutScript);
    }
    let paid = payout.value.to_sat();
    if paid < expected_payout_amount_sats {
        return Err(SilentPaymentProofError::PayoutAmount {
            paid,
            expected: expected_payout_amount_sats,
        });
    }
    Ok(())
}

fn tagged_hash(tag: &str, parts: &[&[u8]]) -> [u8; 32] {
    let tag_hash = sha256::Hash::hash(tag.as_bytes());
    let mut engine = sha256::Hash::engine();
    engine.input(tag_hash.as_ref());
    engine.input(tag_hash.as_ref());
    for part in parts {
        engine.input(part);
    }
    sha256::Hash::from_engine(engine).to_byte_array()
}

fn generator() -> PublicKey {
    let mut compressed = [0x02; 33];
    compressed[1..].copy_from_slice(&GENERATOR_X);
    PublicKey::from_slice(&compressed).expect("the secp256k1 generator is a valid point")
}

#[cfg(test)]
mod tests;
