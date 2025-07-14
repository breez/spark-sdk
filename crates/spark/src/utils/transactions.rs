use bitcoin::{
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    absolute::LockTime, key::Secp256k1, secp256k1::PublicKey, transaction::Version,
};

use crate::core::validate_sequence;

fn create_spark_tx(previous_output: OutPoint, sequence: Sequence, output: TxOut) -> Transaction {
    Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            sequence,
            ..Default::default()
        }],
        output: vec![output, ephemeral_anchor_output()],
    }
}

pub fn create_node_tx(
    sequence: Sequence,
    parent_outpoint: OutPoint,
    value: Amount,
    script_pubkey: ScriptBuf,
) -> Transaction {
    create_spark_tx(
        parent_outpoint,
        sequence,
        TxOut {
            value,
            script_pubkey,
        },
    )
}

pub fn create_refund_tx(
    sequence: Sequence,
    node_outpoint: OutPoint,
    amount_sat: u64,
    receiving_pubkey: &PublicKey,
    network: impl Into<Network>,
) -> Transaction {
    let script_pubkey = script_pubkey_from_pubkey(receiving_pubkey, network);

    create_spark_tx(
        node_outpoint,
        sequence,
        TxOut {
            value: Amount::from_sat(amount_sat),
            script_pubkey,
        },
    )
}

fn ephemeral_anchor_output() -> TxOut {
    TxOut {
        script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]), // Pay-to-anchor (P2A) ephemeral anchor output
        value: Amount::from_sat(0),
    }
}

fn script_pubkey_from_pubkey(pubkey: &PublicKey, network: impl Into<Network>) -> ScriptBuf {
    // TODO: Isolate secp256k1 initialization to avoid multiple initializations
    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    Address::p2tr(&secp, pubkey.x_only_public_key().0, None, network).script_pubkey()
}

pub fn validate_unsigned_refund_tx(
    refund_tx: &Transaction,
    network: impl Into<bitcoin::Network>,
    amount: Option<u64>,
    destination: Option<PublicKey>,
    parent: Option<Transaction>,
) -> Result<(), TransactionValidationError> {
    if refund_tx.output.len() < 2 {
        return Err(TransactionValidationError::MissingOutput);
    }

    if refund_tx.output.len() > 2 {
        return Err(TransactionValidationError::TooManyOutputs);
    }

    if refund_tx.input.len() < 1 {
        return Err(TransactionValidationError::MissingInput);
    }

    if refund_tx.input.len() > 1 {
        return Err(TransactionValidationError::TooManyInputs);
    }

    let input = &refund_tx.input[0];
    if !validate_sequence(input.sequence) {
        return Err(TransactionValidationError::InvalidInputSequence(
            input.sequence,
        ));
    }

    let first_output = &refund_tx.output[0];
    let second_output = &refund_tx.output[1];
    if second_output != &ephemeral_anchor_output() {
        return Err(TransactionValidationError::InvalidOutput);
    }

    if let Some(expected_amount) = amount {
        if first_output.value != Amount::from_sat(expected_amount) {
            return Err(TransactionValidationError::InvalidAmount);
        }
    }

    if let Some(expected_destination) = destination {
        if first_output.script_pubkey != script_pubkey_from_pubkey(&expected_destination, network) {
            return Err(TransactionValidationError::InvalidOutput);
        }
    }

    if let Some(parent_tx) = parent {
        if input.previous_output.txid != parent_tx.compute_txid() || input.previous_output.vout != 0
        {
            return Err(TransactionValidationError::InvalidParent);
        }
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionValidationError {
    #[error("Missing input")]
    MissingInput,
    #[error("Missing output")]
    MissingOutput,
    #[error("Invalid amount")]
    InvalidAmount,
    #[error("Invalid input sequence: {0}")]
    InvalidInputSequence(Sequence),
    #[error("Invalid output")]
    InvalidOutput,
    #[error("Invalid parent")]
    InvalidParent,
    #[error("Too many inputs")]
    TooManyInputs,
    #[error("Too many outputs")]
    TooManyOutputs,
}
