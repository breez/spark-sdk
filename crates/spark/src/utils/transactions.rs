use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
    key::Secp256k1, secp256k1::PublicKey, transaction::Version,
};

use crate::Network;

const ESTIMATED_TX_SIZE: u64 = 191;
const DEFAULT_FEE_RATE: u64 = 5;
const DEFAULT_FEE_SATS: u64 = ESTIMATED_TX_SIZE * DEFAULT_FEE_RATE;

pub(crate) struct NodeTransactions {
    pub cpfp_tx: Transaction,
    pub direct_tx: Option<Transaction>,
}

pub(crate) struct RefundTransactions {
    pub cpfp_tx: Transaction,
    pub direct_tx: Option<Transaction>,
    pub direct_from_cpfp_tx: Option<Transaction>,
}

pub(crate) struct ConnectorRefundTxsParams<'a> {
    pub cpfp_sequence: Sequence,
    pub direct_sequence: Sequence,
    pub cpfp_outpoint: OutPoint,
    pub direct_outpoint: Option<OutPoint>,
    pub connector_outpoint: OutPoint,
    pub amount_sats: u64,
    pub receiving_pubkey: &'a PublicKey,
    pub network: Network,
}

/// Creates a Bitcoin transaction for the Spark protocol with customizable parameters.
///
/// This function builds a transaction with a single input and one or two outputs:
/// - The main output pays to the provided script with the specified value
/// - An optional anchor output (when `include_anchor` is true)
///
/// # Arguments
///
/// * `previous_output` - The outpoint to use as the input for this transaction
/// * `sequence` - The sequence number to use for the input (used for timelocks)
/// * `value` - The amount to send in the transaction
/// * `script_pubkey` - The output script to pay to
/// * `apply_fee` - Whether to subtract a fee from the value (using `DEFAULT_FEE_SATS`)
/// * `include_anchor` - Whether to include an ephemeral anchor output (for CPFP)
pub(crate) fn create_spark_tx(
    previous_output: OutPoint,
    sequence: Sequence,
    value: Amount,
    script_pubkey: ScriptBuf,
    apply_fee: bool,
    include_anchor: bool,
) -> Transaction {
    let value = if apply_fee {
        maybe_apply_fee(value.to_sat())
    } else {
        value.to_sat()
    };

    let mut tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            sequence,
            ..Default::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey,
        }],
    };

    if include_anchor {
        tx.output.push(ephemeral_anchor_output());
    }

    tx
}

/// Creates a pair of transactions for a Spark node, a CPFP transaction and an optional direct transaction.
///
/// This function generates two types of transactions:
/// 1. A CPFP (Child Pays For Parent) transaction that always includes an anchor output for fee bumping
/// 2. An optional direct transaction that can be used for direct spending (if `direct_outpoint` is provided)
///
/// The CPFP transaction is to be broadcast by the user in case of a unilateral exit. The direct transaction
/// is to be used by the watchtower to be broadcast on the user's behalf if in case of an attack while the
/// user is offline and unable to broadcast the CPFP transaction themselves. The sequence number for the
/// direct transaction is always `DIRECT_TIME_LOCK_OFFSET` blocks higher than the CPFP transaction so that
/// the CPFP transaction can be broadcast first.
///
/// # Arguments
///
/// * `cpfp_sequence` - The sequence number to use for the CPFP transaction's input
/// * `direct_sequence` - The sequence number to use for the direct transaction's input (if created)
/// * `cpfp_outpoint` - The outpoint to spend in the CPFP transaction
/// * `direct_outpoint` - Optional outpoint to spend in the direct transaction
/// * `value` - The amount to send in both transactions
/// * `script_pubkey` - The output script to pay to in both transactions
/// * `apply_fee` - Whether to subtract a fee from the direct transaction (fees are not applied to CPFP tx)
///
/// # Returns
///
/// A `NodeTransactions` struct containing:
/// - `cpfp_tx`: Always present, includes an anchor output
/// - `direct_tx`: Only present if `direct_outpoint` is provided, has no anchor output
pub(crate) fn create_node_txs(
    cpfp_sequence: Sequence,
    direct_sequence: Sequence,
    cpfp_outpoint: OutPoint,
    direct_outpoint: Option<OutPoint>,
    value: Amount,
    script_pubkey: ScriptBuf,
    apply_fee: bool,
) -> NodeTransactions {
    let cpfp_tx = create_spark_tx(
        cpfp_outpoint,
        cpfp_sequence,
        value,
        script_pubkey.clone(),
        false,
        true,
    );
    let direct_tx = direct_outpoint.map(|outpoint| {
        create_spark_tx(
            outpoint,
            direct_sequence,
            value,
            script_pubkey,
            apply_fee,
            false,
        )
    });

    NodeTransactions { cpfp_tx, direct_tx }
}

/// Creates a set of refund transactions that can be used to claim funds in case of protocol failures.
///
/// This function generates three possible transactions:
/// 1. A CPFP (Child Pays For Parent) transaction that always includes an anchor output
/// 2. An optional direct transaction that spends from the direct outpoint (if provided)
/// 3. An optional direct transaction that spends from the CPFP outpoint, but with a different
///    sequence number (used as an alternative spending path)
///
/// The CPFP refund transaction is to be broadcast by the user in case of a unilateral exit. The direct
/// refund transactions are to be used by the watchtower to be broadcast on the user's behalf if in case
/// of an attack while the user is offline and unable to broadcast the CPFP refund transaction themselves.
/// The sequence number for the direct transaction is always `DIRECT_TIME_LOCK_OFFSET` blocks higher than
/// the CPFP refund transaction so that the CPFP refund transaction can be broadcast first.
///
/// All transactions pay to a P2TR (Pay-to-Taproot) address derived from the provided public key.
///
/// # Arguments
///
/// * `cpfp_sequence` - The sequence number to use for the CPFP transaction's input
/// * `direct_sequence` - The sequence number to use for direct transactions' inputs
/// * `cpfp_outpoint` - The outpoint to spend in the CPFP transaction
/// * `direct_outpoint` - Optional outpoint to spend in the direct transaction
/// * `amount_sat` - The amount in satoshis to send in the transactions
/// * `receiving_pubkey` - The public key to send the funds to (used to create P2TR address)
/// * `network` - The Bitcoin network to use (affects address format)
///
/// # Returns
///
/// A `RefundTransactions` struct containing:
/// - `cpfp_tx`: Always present, includes an anchor output
/// - `direct_tx`: Only present if `direct_outpoint` is provided
/// - `direct_from_cpfp_tx`: Alternative transaction that spends from the CPFP outpoint
///   with the direct sequence number (only present if `direct_outpoint` is provided)
pub(crate) fn create_refund_txs(
    cpfp_sequence: Sequence,
    direct_sequence: Sequence,
    cpfp_outpoint: OutPoint,
    direct_outpoint: Option<OutPoint>,
    amount_sat: u64,
    receiving_pubkey: &PublicKey,
    network: Network,
) -> RefundTransactions {
    // TODO: Isolate secp256k1 initialization to avoid multiple initializations
    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    let value = Amount::from_sat(amount_sat);
    let addr = Address::p2tr(&secp, receiving_pubkey.x_only_public_key().0, None, network);

    let cpfp_tx = create_spark_tx(
        cpfp_outpoint,
        cpfp_sequence,
        value,
        addr.script_pubkey(),
        false,
        true,
    );

    let direct_tx = direct_outpoint.map(|outpoint| {
        create_spark_tx(
            outpoint,
            direct_sequence,
            value,
            addr.script_pubkey(),
            true,
            false,
        )
    });

    let direct_from_cpfp_tx = direct_outpoint.map(|_| {
        create_spark_tx(
            cpfp_outpoint,
            direct_sequence,
            value,
            addr.script_pubkey(),
            true,
            false,
        )
    });

    RefundTransactions {
        cpfp_tx,
        direct_tx,
        direct_from_cpfp_tx,
    }
}

/// Creates a set of refund transactions for a connector in the Spark protocol.
///
/// This function is similar to `create_refund_txs`, but specifically designed for connectors.
/// It generates transactions that spend from both the connector outpoint and one of the node
/// outpoints in a single transaction. This is important for refund scenarios where both
/// inputs need to be spent together.
///
/// The function generates three possible transactions:
/// 1. A CPFP transaction that spends from both the CPFP outpoint and connector outpoint
/// 2. An optional direct transaction that spends from both the direct outpoint and connector outpoint (if provided)
/// 3. An optional alternative direct transaction that spends from both the CPFP outpoint and connector outpoint,
///    but using the direct sequence number (if direct_outpoint is provided)
///
/// All transactions pay to a P2TR (Pay-to-Taproot) address derived from the provided public key.
///
/// # Arguments
///
/// * `params` - A `ConnectorRefundTxsParams` struct containing:
///   - `cpfp_sequence`: The sequence number for the CPFP transaction
///   - `direct_sequence`: The sequence number for direct transactions
///   - `cpfp_outpoint`: The CPFP outpoint to spend
///   - `direct_outpoint`: Optional direct outpoint to spend
///   - `connector_outpoint`: The connector's outpoint that must be spent along with node outpoints
///   - `amount_sats`: The amount in satoshis to send
///   - `receiving_pubkey`: The public key to send funds to (used to create P2TR address)
///   - `network`: The Bitcoin network to use (affects address format)
///
/// # Returns
///
/// A `RefundTransactions` struct containing:
/// - `cpfp_tx`: Always present, spends both CPFP and connector outpoints
/// - `direct_tx`: Only present if `direct_outpoint` is provided, spends direct and connector outpoints
/// - `direct_from_cpfp_tx`: Alternative transaction that spends CPFP and connector outpoints with
///   the direct sequence number (only present if `direct_outpoint` is provided)
pub(crate) fn create_connector_refund_txs(
    params: ConnectorRefundTxsParams<'_>,
) -> RefundTransactions {
    // TODO: Isolate secp256k1 initialization to avoid multiple initializations
    let secp = Secp256k1::new();
    let network: bitcoin::Network = params.network.into();
    let value = Amount::from_sat(params.amount_sats);
    let addr = Address::p2tr(
        &secp,
        params.receiving_pubkey.x_only_public_key().0,
        None,
        network,
    );

    let mut cpfp_tx = create_spark_tx(
        params.cpfp_outpoint,
        params.cpfp_sequence,
        value,
        addr.script_pubkey(),
        false,
        false,
    );
    cpfp_tx.input.push(TxIn {
        previous_output: params.connector_outpoint,
        ..Default::default()
    });

    let direct_tx = params.direct_outpoint.map(|outpoint| {
        let mut tx = create_spark_tx(
            outpoint,
            params.direct_sequence,
            value,
            addr.script_pubkey(),
            true,
            false,
        );
        tx.input.push(TxIn {
            previous_output: params.connector_outpoint,
            ..Default::default()
        });
        tx
    });

    let direct_from_cpfp_tx = params.direct_outpoint.map(|_| {
        let mut tx = create_spark_tx(
            params.cpfp_outpoint,
            params.direct_sequence,
            value,
            addr.script_pubkey(),
            true,
            false,
        );
        tx.input.push(TxIn {
            previous_output: params.connector_outpoint,
            ..Default::default()
        });
        tx
    });

    RefundTransactions {
        cpfp_tx,
        direct_tx,
        direct_from_cpfp_tx,
    }
}

pub(crate) fn create_static_deposit_refund_tx(
    deposit_outpoint: OutPoint,
    amount_sat: u64,
    refund_address: &Address,
) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: deposit_outpoint,
            ..Default::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(amount_sat),
            script_pubkey: refund_address.script_pubkey(),
        }],
    }
}

fn ephemeral_anchor_output() -> TxOut {
    TxOut {
        script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]), // Pay-to-anchor (P2A) ephemeral anchor output
        value: Amount::from_sat(0),
    }
}

pub fn is_ephemeral_anchor_output(tx_out: &TxOut) -> bool {
    tx_out.value.to_sat() == 0 && tx_out.script_pubkey.as_bytes() == [0x51, 0x02, 0x4e, 0x73]
}

fn maybe_apply_fee(amount_sats: u64) -> u64 {
    if amount_sats > DEFAULT_FEE_SATS {
        amount_sats.saturating_sub(DEFAULT_FEE_SATS)
    } else {
        amount_sats
    }
}
