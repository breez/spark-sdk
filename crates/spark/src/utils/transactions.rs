use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
    key::Secp256k1, secp256k1::PublicKey, transaction::Version,
};

use crate::Network;

const ESTIMATED_TX_SIZE: u64 = 191;
const DEFAULT_FEE_RATE: u64 = 5;
const DEFAULT_FEE_SATS: u64 = ESTIMATED_TX_SIZE * DEFAULT_FEE_RATE;

pub struct NodeTransactions {
    pub cpfp_tx: Transaction,
    pub direct_tx: Option<Transaction>,
}

pub struct RefundTransactions {
    pub cpfp_tx: Transaction,
    pub direct_tx: Option<Transaction>,
    pub direct_from_cpfp_tx: Option<Transaction>,
}

pub struct ConnectorRefundTxsParams<'a> {
    pub cpfp_sequence: Sequence,
    pub direct_sequence: Sequence,
    pub cpfp_outpoint: OutPoint,
    pub direct_outpoint: Option<OutPoint>,
    pub connector_outpoint: OutPoint,
    pub amount_sats: u64,
    pub receiving_pubkey: &'a PublicKey,
    pub network: Network,
}

fn create_spark_tx(
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

pub fn create_node_txs(
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

pub fn create_refund_txs(
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

pub fn create_connector_refund_txs(params: ConnectorRefundTxsParams<'_>) -> RefundTransactions {
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

pub fn create_static_deposit_refund_tx(
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

fn maybe_apply_fee(amount_sats: u64) -> u64 {
    if amount_sats > DEFAULT_FEE_SATS {
        amount_sats.saturating_sub(DEFAULT_FEE_SATS)
    } else {
        amount_sats
    }
}
