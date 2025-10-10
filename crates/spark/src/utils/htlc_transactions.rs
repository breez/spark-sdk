use std::str::FromStr;

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, XOnlyPublicKey,
    hashes::{Hash, sha256},
    opcodes::all::*,
    script::Builder,
    secp256k1::{PublicKey, Secp256k1},
    taproot::TaprootBuilder,
};

use crate::{
    Network,
    signer::SignerError,
    utils::transactions::{RefundTransactions, create_spark_tx},
};

const LIGHTNING_HTLC_TIME_LOCK: u16 = 2160;
const UNSPENDABLE_PUBKEY: &str =
    "0250929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";

struct CreateLightningHtlcTxsParams<'a> {
    /// The outpoint to use as the input for this transaction
    pub previous_output: OutPoint,
    /// The sequence number to use for the input
    pub sequence: Sequence,
    /// The amount to send in the transaction
    pub value: Amount,
    /// The payment hash for the hash-lock condition
    pub hash: &'a sha256::Hash,
    /// The public key for the hash-lock spending path
    pub hash_lock_pubkey: &'a PublicKey,
    /// The public key for the time-lock spending path
    pub sequence_lock_pubkey: &'a PublicKey,
    /// The Bitcoin network to use (affects address format)
    pub network: Network,
    /// Whether to subtract a fee from the value (using `DEFAULT_FEE_SATS`)
    pub apply_fee: bool,
    /// Whether to include an ephemeral anchor output (for CPFP)
    pub include_anchor: bool,
}

pub struct CreateLightningHtlcRefundTxsParams<'a> {
    /// The sequence number to use for the CPFP transaction's input
    pub cpfp_sequence: Sequence,
    /// The sequence number to use for direct transactions' inputs
    pub direct_sequence: Sequence,
    /// The outpoint to spend in the CPFP transaction
    pub cpfp_outpoint: OutPoint,
    /// Optional outpoint to spend in the direct transaction
    pub direct_outpoint: Option<OutPoint>,
    /// The amount in satoshis to send in the transactions
    pub amount_sat: u64,
    /// The payment hash for the hash-lock condition
    pub hash: &'a sha256::Hash,
    /// The public key for the hash-lock spending path
    pub hash_lock_pubkey: &'a PublicKey,
    /// The public key for the time-lock spending path
    pub sequence_lock_pubkey: &'a PublicKey,
    /// The Bitcoin network to use (affects address format)
    pub network: Network,
}

/// Creates a Lightning-style Hash Time-Locked Contract (HTLC) transaction
///
/// This function creates a transaction that spends to a taproot address with two spending paths:
/// 1. A hash lock path requiring the correct preimage and a signature
/// 2. A time lock path allowing spending after a set time (default 2160 blocks)
///
/// # Arguments
///
/// * `params` - Parameters for creating the HTLC transaction, including:
///   - `previous_output`: The outpoint to use as the input for this transaction
///   - `sequence`: The sequence number to use for the input (used for timelocks)
///   - `value`: The amount to send in the transaction
///   - `hash`: The payment hash for the hash-lock condition
///   - `hash_lock_pubkey`: The public key for the hash-lock spending path
///   - `sequence_lock_pubkey`: The public key for the time-lock spending path
///   - `network`: The Bitcoin network to use
///   - `apply_fee`: Whether to subtract a fee from the value (using `DEFAULT_FEE_SATS`)
///   - `include_anchor`: Whether to include an ephemeral anchor output (for CPFP)
fn create_lightning_htlc_tx(
    params: CreateLightningHtlcTxsParams<'_>,
) -> Result<Transaction, SignerError> {
    let CreateLightningHtlcTxsParams {
        previous_output,
        sequence,
        value,
        hash,
        hash_lock_pubkey,
        sequence_lock_pubkey,
        network,
        apply_fee,
        include_anchor,
    } = params;
    let script_pubkey = create_htlc_taproot_address(
        hash,
        hash_lock_pubkey,
        Sequence::from_height(LIGHTNING_HTLC_TIME_LOCK),
        sequence_lock_pubkey,
        network,
    )?;

    Ok(create_spark_tx(
        previous_output,
        sequence,
        value,
        script_pubkey,
        apply_fee,
        include_anchor,
    ))
}

/// Creates a set of Lightning-style Hash Time-Locked Contract (HTLC) refund transactions
///
/// This function generates refund transactions for different scenarios using HTLC-based
/// taproot addresses with two spending paths:
/// 1. A hash lock path requiring the correct preimage and a signature
/// 2. A time lock path allowing spending after a set timelock period
///
/// It creates three possible transactions:
/// - A CPFP (Child Pays For Parent) transaction that always includes an anchor output
/// - An optional direct transaction that spends from the direct outpoint (if provided)
/// - An optional transaction that spends from the CPFP outpoint but with the direct sequence number
///
/// The CPFP refund transaction is designed for user-initiated unilateral exits. The direct
/// refund transactions are for watchtowers to broadcast on behalf of offline users in case of
/// attack. The direct transaction has a higher timelock than the CPFP refund transaction
/// to prioritize the CPFP refund path.
///
/// All transactions pay to a P2TR (Pay-to-Taproot) address derived from the provided parameters.
///
/// # Arguments
///
/// * `params` - Parameters for creating the refund transactions, including:
///   - `cpfp_sequence`: The sequence number to use for the CPFP transaction's input
///   - `direct_sequence`: The sequence number to use for direct transactions' inputs
///   - `cpfp_outpoint`: The outpoint to spend in the CPFP transaction
///   - `direct_outpoint`: Optional outpoint to spend in the direct transaction
///   - `amount_sat`: The amount in satoshis to send in the transactions
///   - `hash`: The payment hash for the hash-lock condition
///   - `hash_lock_pubkey`: The public key for the hash-lock spending path
///   - `sequence_lock_pubkey`: The public key for the time-lock spending path
///   - `network`: The Bitcoin network to use (affects address format)
///
/// # Returns
///
/// A `RefundTransactions` struct containing:
/// - `cpfp_tx`: Always present, includes an anchor output
/// - `direct_tx`: Only present if `direct_outpoint` is provided
/// - `direct_from_cpfp_tx`: Alternative transaction that spends from the CPFP outpoint
///   with the direct sequence number (only present if `direct_outpoint` is provided)
pub(crate) fn create_lightning_htlc_refund_txs(
    params: CreateLightningHtlcRefundTxsParams<'_>,
) -> Result<RefundTransactions, SignerError> {
    let CreateLightningHtlcRefundTxsParams {
        cpfp_sequence,
        direct_sequence,
        cpfp_outpoint,
        direct_outpoint,
        amount_sat,
        hash,
        hash_lock_pubkey,
        sequence_lock_pubkey,
        network,
    } = params;
    let value = Amount::from_sat(amount_sat);

    let cpfp_tx = create_lightning_htlc_tx(CreateLightningHtlcTxsParams {
        previous_output: cpfp_outpoint,
        sequence: cpfp_sequence,
        value,
        hash,
        hash_lock_pubkey,
        sequence_lock_pubkey,
        network,
        apply_fee: false,
        include_anchor: true,
    })?;

    let direct_tx = direct_outpoint
        .map(|outpoint| {
            create_lightning_htlc_tx(CreateLightningHtlcTxsParams {
                previous_output: outpoint,
                sequence: direct_sequence,
                value,
                hash,
                hash_lock_pubkey,
                sequence_lock_pubkey,
                network,
                apply_fee: true,
                include_anchor: false,
            })
        })
        .transpose()?;

    let direct_from_cpfp_tx = direct_outpoint
        .map(|_| {
            create_lightning_htlc_tx(CreateLightningHtlcTxsParams {
                previous_output: cpfp_outpoint,
                sequence: direct_sequence,
                value,
                hash,
                hash_lock_pubkey,
                sequence_lock_pubkey,
                network,
                apply_fee: true,
                include_anchor: false,
            })
        })
        .transpose()?;

    Ok(RefundTransactions {
        cpfp_tx,
        direct_tx,
        direct_from_cpfp_tx,
    })
}

/// Creates a taproot-based HTLC address with hash lock and sequence lock spending paths
///
/// This function creates a taproot address with two spending paths:
/// 1. A hash lock path that allows spending if the correct hash preimage is provided
/// 2. A sequence lock path that allows spending after a timelock has expired
///
/// # Arguments
///
/// * `hash` - The hash to use for the hash lock script
/// * `hash_lock_pubkey` - The public key that can spend using the hash lock path
/// * `sequence` - The sequence number for the timelock
/// * `sequence_lock_pubkey` - The public key that can spend after the timelock
/// * `network` - The Bitcoin network to use for the address
pub fn create_htlc_taproot_address(
    hash: &sha256::Hash,
    hash_lock_pubkey: &PublicKey,
    sequence: Sequence,
    sequence_lock_pubkey: &PublicKey,
    network: Network,
) -> Result<ScriptBuf, SignerError> {
    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    let unspendable_pubkey = XOnlyPublicKey::from(
        PublicKey::from_str(UNSPENDABLE_PUBKEY).map_err(|_| SignerError::UnknownKey)?,
    );

    // Create hash lock and sequence lock scripts
    let hash_lock_script = create_hash_lock_script(hash, hash_lock_pubkey);
    let sequence_lock_script = create_sequence_lock_script(sequence, sequence_lock_pubkey);

    // Build a taproot merkle root with both scripts
    let merkle_root = TaprootBuilder::new()
        .add_leaf(1, hash_lock_script)
        .map_err(|e| SignerError::TaprootBuilderError(format!("Error adding hash lock leaf: {e}")))?
        .add_leaf(1, sequence_lock_script)
        .map_err(|e| {
            SignerError::TaprootBuilderError(format!("Error adding sequence lock leaf: {e}"))
        })?
        .finalize(&secp, unspendable_pubkey)
        .map_err(|_| SignerError::TaprootBuilderError("Error finalizing taproot tree".to_string()))?
        .merkle_root();
    let address = Address::p2tr(&secp, unspendable_pubkey, merkle_root, network);

    Ok(address.script_pubkey())
}

/// Creates a hash lock script for an HTLC transaction
///
/// This function builds a script that verifies a SHA256 hash and checks a signature:
/// `SHA256 <hash> EQUALVERIFY <pubkey> CHECKSIG`
///
/// # Arguments
///
/// * `hash` - The hash preimage to lock the funds with
/// * `hash_lock_pubkey` - The public key that can spend the output if the preimage is provided
pub fn create_hash_lock_script(hash: &sha256::Hash, hash_lock_pubkey: &PublicKey) -> ScriptBuf {
    Builder::new()
        .push_opcode(OP_SHA256)
        .push_slice(hash.to_byte_array())
        .push_opcode(OP_EQUALVERIFY)
        .push_x_only_key(&hash_lock_pubkey.x_only_public_key().0)
        .push_opcode(OP_CHECKSIG)
        .into_script()
}

/// Creates a sequence lock script for an HTLC transaction
///
/// This function builds a script that verifies a timelock and checks a signature:
/// `<sequence> CSV DROP <pubkey> CHECKSIG`
///
/// # Arguments
///
/// * `sequence` - The sequence number for the timelock
/// * `sequence_lock_pubkey` - The public key that can spend after the timelock expires
pub fn create_sequence_lock_script(
    sequence: Sequence,
    sequence_lock_pubkey: &PublicKey,
) -> ScriptBuf {
    Builder::new()
        .push_int(sequence.to_consensus_u32() as i64)
        .push_opcode(OP_CSV)
        .push_opcode(OP_DROP)
        .push_x_only_key(&sequence_lock_pubkey.x_only_public_key().0)
        .push_opcode(OP_CHECKSIG)
        .into_script()
}
