use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime, key::Secp256k1, secp256k1::PublicKey, transaction::Version,
};

use crate::{Network, utils::anchor::ephemeral_anchor_output};

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

    let new_refund_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: node_outpoint,
            script_sig: ScriptBuf::default(),
            sequence,
            witness: Witness::default(),
        }],
        output: vec![
            TxOut {
                value: Amount::from_sat(amount_sat),
                script_pubkey: addr.script_pubkey(),
            },
            ephemeral_anchor_output(),
        ],
    };

    new_refund_tx
}
