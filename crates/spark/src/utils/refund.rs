use std::collections::BTreeMap;

use crate::{
    Network, bitcoin::sighash_from_tx, core::next_sequence, services::LeafKeyTweak, tree::TreeNode,
};
use bitcoin::Transaction;
use bitcoin::absolute::LockTime;
use bitcoin::blockdata::transaction::Version;
use bitcoin::hashes::Hash;
use bitcoin::{key::Secp256k1, secp256k1::PublicKey};
use frost_core::round2::SignatureShare;
use frost_secp256k1_tr::round1::SigningCommitments;

use frost_secp256k1_tr::{Identifier, Secp256K1Sha256TR};

use crate::signer::Signer;
use crate::signer::SignerError;

pub struct SignedTx {
    pub node_id: String,
    pub signing_public_key: PublicKey,
    pub tx: Transaction,
    pub user_signature: SignatureShare<Secp256K1Sha256TR>,
    pub signing_commitments: BTreeMap<Identifier, SigningCommitments>,
    pub user_signature_commitment: SigningCommitments,
    pub network: Network,
}

pub fn create_refund_tx(
    leaf: &TreeNode,
    receiving_pubkey: &PublicKey,
    network: Network,
) -> Result<bitcoin::Transaction, SignerError> {
    let node_tx = leaf.node_tx.clone();
    let refund_tx = leaf.refund_tx.clone();

    let mut new_refund_tx = bitcoin::Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![],
        output: vec![],
    };

    let old_sequence = refund_tx.input[0].sequence;
    let sequence = next_sequence(old_sequence).unwrap_or_default();

    new_refund_tx.input.push(bitcoin::TxIn {
        previous_output: bitcoin::OutPoint {
            txid: node_tx.compute_txid(),
            vout: 0,
        },
        script_sig: bitcoin::ScriptBuf::default(),
        sequence,
        witness: bitcoin::Witness::default(),
    });

    let secp = Secp256k1::new();
    let network: bitcoin::Network = network.into();
    let addr = bitcoin::Address::p2tr(&secp, receiving_pubkey.x_only_public_key().0, None, network);

    new_refund_tx.output.push(bitcoin::TxOut {
        value: node_tx.output[0].value,
        script_pubkey: addr.script_pubkey(),
    });

    Ok(new_refund_tx)
}

pub async fn sign_refunds<S: Signer>(
    signer: &S,
    leaves: &Vec<LeafKeyTweak>,
    spark_commitments: Vec<BTreeMap<Identifier, SigningCommitments>>,
    receiver_pubkey: &PublicKey,
    network: Network,
) -> Result<Vec<SignedTx>, SignerError> {
    // sign refunds. TODO: In JS SDK, this is the `sign_refunds` function
    let mut signed_refunds = Vec::with_capacity(leaves.len());

    for (i, leaf) in leaves.iter().enumerate() {
        let node_tx = leaf.node.node_tx.clone();

        let new_refund_tx = create_refund_tx(&leaf.node, receiver_pubkey, network)?;

        let sighash = sighash_from_tx(&new_refund_tx, 0, &node_tx.output[0])
            .map_err(|e| SignerError::Generic(e.to_string()))?;

        let self_commitment = signer.generate_frost_signing_commitments().await?;
        let spark_commitment = spark_commitments[i].clone();

        let user_signature_share = signer
            .sign_frost(
                sighash.to_raw_hash().to_byte_array().as_ref(),
                &leaf.signing_public_key,
                &leaf.signing_public_key,
                &leaf.node.verifying_public_key,
                &self_commitment,
                spark_commitment.clone(),
                None,
            )
            .await?;

        signed_refunds.push(SignedTx {
            node_id: leaf.node.id.clone(),
            signing_public_key: leaf.signing_public_key.clone(),
            tx: new_refund_tx,
            user_signature: user_signature_share,
            user_signature_commitment: self_commitment,
            signing_commitments: spark_commitment,
            network,
        });
    }

    Ok(signed_refunds)
}
