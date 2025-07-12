use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
    key::Secp256k1, secp256k1::PublicKey, transaction::Version,
};

use crate::Network;

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
    network: Network,
) -> Transaction {
    // TODO: Isolate secp256k1 initialization to avoid multiple initializations
    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    let addr = Address::p2tr(&secp, receiving_pubkey.x_only_public_key().0, None, network);

    create_spark_tx(
        node_outpoint,
        sequence,
        TxOut {
            value: Amount::from_sat(amount_sat),
            script_pubkey: addr.script_pubkey(),
        },
    )
}

fn ephemeral_anchor_output() -> TxOut {
    TxOut {
        script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]), // Pay-to-anchor (P2A) ephemeral anchor output
        value: Amount::from_sat(0),
    }
}
