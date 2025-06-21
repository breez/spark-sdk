use std::collections::BTreeMap;

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut,
    absolute::LockTime,
    address::NetworkUnchecked,
    consensus::{deserialize, serialize},
    hashes::{Hash, sha256},
    secp256k1::{Message, PublicKey, ecdsa, schnorr},
    transaction::Version,
};
use frost_core::round1::NonceCommitment;
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments, round2::SignatureShare};

use crate::{
    Network,
    bitcoin::{BitcoinService, sighash_from_tx},
    core::initial_sequence,
    operator::{OperatorPool, rpc::SparkRpcClient},
    services::DepositServiceError,
    signer::Signer,
    tree::{SigningKeyshare, TreeNode},
};
use spark_protos::{
    common::{self, SignatureIntent},
    spark::{
        self, FinalizeNodeSignaturesRequest, GenerateDepositAddressRequest, NodeSignatures,
        SigningJob, StartDepositTreeCreationRequest,
    },
};
pub struct DepositService<S>
where
    S: Signer,
{
    bitcoin_service: BitcoinService,
    client: SparkRpcClient<S>,
    identity_public_key: PublicKey,
    network: Network,
    operator_pool: OperatorPool,
    signer: S,
}

pub struct DepositAddress {
    pub address: Address,
    pub leaf_id: String,
    pub user_signing_public_key: PublicKey,
    pub verifying_public_key: PublicKey,
}

impl<S> DepositService<S>
where
    S: Signer,
{
    fn spark_network(&self) -> spark_protos::spark::Network {
        self.network.into()
    }

    pub fn new(
        bitcoin_service: BitcoinService,
        client: SparkRpcClient<S>,
        identity_public_key: PublicKey,
        network: impl Into<Network>,
        operator_pool: OperatorPool,
        signer: S,
    ) -> Self {
        DepositService {
            bitcoin_service,
            client,
            identity_public_key,
            network: network.into(),
            operator_pool,
            signer,
        }
    }

    pub async fn create_tree_root(
        &self,
        signing_public_key: &PublicKey,
        verifying_public_key: &PublicKey,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, DepositServiceError> {
        let deposit_txid = deposit_tx.compute_txid();
        let deposit_output = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(DepositServiceError::InvalidOutputIndex)?;
        let deposit_value = deposit_output.value;

        let root_tx = Transaction {
            version: Version(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: deposit_txid,
                    vout,
                },
                script_sig: ScriptBuf::new(),
                sequence: Default::default(),
                witness: Default::default(),
            }],
            output: vec![
                TxOut {
                    script_pubkey: deposit_output.script_pubkey.clone(),
                    value: deposit_value,
                },
                ephemeral_anchor_output(),
            ],
        };

        // Get random signing commitment for root nonce
        let root_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;

        // Calculate sighash for root transaction
        let root_tx_sighash = sighash_from_tx(&root_tx, 0, deposit_output)?;
        let root_txid = root_tx.compute_txid();

        // Create refund transaction
        let refund_address = self
            .bitcoin_service
            .p2tr_address(signing_public_key.x_only_public_key().0, None);
        let refund_script = refund_address.script_pubkey();

        let refund_tx = Transaction {
            version: Version(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: root_txid,
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                sequence: initial_sequence(),
                witness: Default::default(),
            }],
            output: vec![
                TxOut {
                    script_pubkey: refund_script,
                    value: deposit_value,
                },
                ephemeral_anchor_output(),
            ],
        };

        // Get random signing commitment for refund nonce
        let refund_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;

        // Calculate sighash for refund transaction
        let refund_tx_sighash = sighash_from_tx(&refund_tx, 0, &root_tx.output[0])?;

        // Get spark client
        let root_tx_bytes = serialize(&root_tx);
        let refund_tx_bytes = serialize(&refund_tx);

        let tree_resp = self
            .client
            .start_deposit_tree_creation(StartDepositTreeCreationRequest {
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                on_chain_utxo: Some(spark::Utxo {
                    raw_tx: serialize(&deposit_tx),
                    vout,
                    network: self.spark_network() as i32,
                    txid: deposit_txid.as_byte_array().to_vec(),
                }),
                root_tx_signing_job: Some(SigningJob {
                    signing_public_key: signing_public_key.serialize().to_vec(),
                    raw_tx: root_tx_bytes.clone(),
                    signing_nonce_commitment: Some(marshal_frost_commitment(
                        &root_nonce_commitment,
                    )?),
                }),
                refund_tx_signing_job: Some(SigningJob {
                    signing_public_key: signing_public_key.serialize().to_vec(),
                    raw_tx: refund_tx_bytes.clone(),
                    signing_nonce_commitment: Some(marshal_frost_commitment(
                        &refund_nonce_commitment,
                    )?),
                }),
            })
            .await?;

        let Some(root_node_signature_shares) = tree_resp.root_node_signature_shares else {
            return Err(DepositServiceError::MissingTreeSignatures);
        };

        let Some(node_tx_signing_result) = root_node_signature_shares.node_tx_signing_result else {
            return Err(DepositServiceError::MissingTreeSignatures);
        };

        let Some(refund_tx_signing_result) = root_node_signature_shares.refund_tx_signing_result
        else {
            return Err(DepositServiceError::MissingTreeSignatures);
        };

        if node_tx_signing_result.signing_nonce_commitments.is_empty() {
            return Err(DepositServiceError::MissingTreeSignatures);
        }

        let mut node_tx_signing_nonce_commitments = BTreeMap::new();
        for (identifier, commitment) in node_tx_signing_result.signing_nonce_commitments {
            let identifier = Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| DepositServiceError::InvalidIdentifier)?,
            )
            .map_err(|_| DepositServiceError::InvalidIdentifier)?;
            let commitments = SigningCommitments::new(
                NonceCommitment::deserialize(&commitment.hiding)
                    .map_err(|_| DepositServiceError::InvalidSignatureShare)?,
                NonceCommitment::deserialize(&commitment.binding)
                    .map_err(|_| DepositServiceError::InvalidSignatureShare)?,
            );
            node_tx_signing_nonce_commitments.insert(identifier, commitments);
        }

        let mut node_tx_signature_shares = BTreeMap::new();
        for (identifier, signature_share) in node_tx_signing_result.signature_shares {
            let identifier = Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| DepositServiceError::InvalidIdentifier)?,
            )
            .map_err(|_| DepositServiceError::InvalidIdentifier)?;
            let signature_share = SignatureShare::deserialize(&signature_share)
                .map_err(|_| DepositServiceError::InvalidSignatureShare)?;
            node_tx_signature_shares.insert(identifier, signature_share);
        }

        let mut node_tx_statechain_public_keys = BTreeMap::new();
        for (identifier, public_key) in node_tx_signing_result.public_keys {
            let identifier = Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| DepositServiceError::InvalidIdentifier)?,
            )
            .map_err(|_| DepositServiceError::InvalidIdentifier)?;
            let public_key = PublicKey::from_slice(&public_key)
                .map_err(|_| DepositServiceError::InvalidPublicKey)?;
            node_tx_statechain_public_keys.insert(identifier, public_key);
        }

        if refund_tx_signing_result
            .signing_nonce_commitments
            .is_empty()
        {
            return Err(DepositServiceError::MissingTreeSignatures);
        }

        let mut refund_tx_signing_nonce_commitments = BTreeMap::new();
        for (identifier, commitment) in refund_tx_signing_result.signing_nonce_commitments {
            let identifier = Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| DepositServiceError::InvalidIdentifier)?,
            )
            .map_err(|_| DepositServiceError::InvalidIdentifier)?;
            let commitments = SigningCommitments::new(
                NonceCommitment::deserialize(&commitment.hiding)
                    .map_err(|_| DepositServiceError::InvalidSignatureShare)?,
                NonceCommitment::deserialize(&commitment.binding)
                    .map_err(|_| DepositServiceError::InvalidSignatureShare)?,
            );
            refund_tx_signing_nonce_commitments.insert(identifier, commitments);
        }

        let mut refund_tx_signature_shares = BTreeMap::new();
        for (identifier, signature_share) in refund_tx_signing_result.signature_shares {
            let identifier = Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| DepositServiceError::InvalidIdentifier)?,
            )
            .map_err(|_| DepositServiceError::InvalidIdentifier)?;
            let signature_share = SignatureShare::deserialize(&signature_share)
                .map_err(|_| DepositServiceError::InvalidSignatureShare)?;
            refund_tx_signature_shares.insert(identifier, signature_share);
        }

        let mut refund_tx_statechain_public_keys = BTreeMap::new();
        for (identifier, public_key) in refund_tx_signing_result.public_keys {
            let identifier = Identifier::deserialize(
                &hex::decode(identifier).map_err(|_| DepositServiceError::InvalidIdentifier)?,
            )
            .map_err(|_| DepositServiceError::InvalidIdentifier)?;
            let public_key = PublicKey::from_slice(&public_key)
                .map_err(|_| DepositServiceError::InvalidPublicKey)?;
            refund_tx_statechain_public_keys.insert(identifier, public_key);
        }

        let tree_resp_verifying_key =
            PublicKey::from_slice(&root_node_signature_shares.verifying_key)
                .map_err(|_| DepositServiceError::InvalidVerifyingKey)?;

        if &tree_resp_verifying_key != verifying_public_key {
            return Err(DepositServiceError::InvalidVerifyingKey);
        }

        let root_sig = self
            .signer
            .sign_frost(
                &root_tx_sighash.to_byte_array(),
                &signing_public_key,
                &signing_public_key,
                &verifying_public_key,
                &root_nonce_commitment,
                node_tx_signing_nonce_commitments.clone(),
                None,
            )
            .await?;
        let refund_sig = self
            .signer
            .sign_frost(
                &refund_tx_sighash.to_byte_array(),
                &signing_public_key,
                &signing_public_key,
                &verifying_public_key,
                &refund_nonce_commitment,
                refund_tx_signing_nonce_commitments.clone(),
                None,
            )
            .await?;

        let root_aggregate = self
            .signer
            .aggregate_frost(
                &root_tx_sighash.to_byte_array(),
                node_tx_signature_shares,
                node_tx_statechain_public_keys,
                &verifying_public_key,
                node_tx_signing_nonce_commitments,
                &root_nonce_commitment,
                &signing_public_key,
                &root_sig,
                None,
            )
            .await?;
        let refund_aggregate = self
            .signer
            .aggregate_frost(
                &root_tx_sighash.to_byte_array(),
                refund_tx_signature_shares,
                refund_tx_statechain_public_keys,
                &verifying_public_key,
                refund_tx_signing_nonce_commitments,
                &refund_nonce_commitment,
                &signing_public_key,
                &refund_sig,
                None,
            )
            .await?;

        let finalize_resp = self
            .client
            .finalize_node_signatures(FinalizeNodeSignaturesRequest {
                intent: SignatureIntent::Creation as i32,
                node_signatures: vec![NodeSignatures {
                    node_id: root_node_signature_shares.node_id,
                    node_tx_signature: root_aggregate
                        .serialize()
                        .map_err(|_| DepositServiceError::InvalidSignatureShare)?
                        .to_vec(),
                    refund_tx_signature: refund_aggregate
                        .serialize()
                        .map_err(|_| DepositServiceError::InvalidSignatureShare)?
                        .to_vec(),
                }],
            })
            .await?;

        let finalized_root_node = finalize_resp.nodes[0].clone();

        // TODO: Insert the finalized root node into the leaf manager
        // TODO: Verify the signatures and store the transactions?

        let signing_keyshare = finalized_root_node
            .signing_keyshare
            .ok_or(DepositServiceError::MissingSigningKeyshare)?;
        let signing_keyshare = SigningKeyshare {
            owner_identifiers: signing_keyshare
                .owner_identifiers
                .into_iter()
                .map(|id| {
                    Ok(Identifier::deserialize(
                        &hex::decode(&id).map_err(|_| DepositServiceError::InvalidIdentifier)?,
                    )
                    .map_err(|_| DepositServiceError::InvalidIdentifier)?)
                })
                .collect::<Result<Vec<_>, DepositServiceError>>()?,
            threshold: signing_keyshare.threshold,
        };

        let nodes = finalize_resp
            .nodes
            .into_iter()
            .map(|node| {
                let signing_keyshare = node
                    .signing_keyshare
                    .ok_or(DepositServiceError::MissingSigningKeyshare)?;
                let signing_keyshare = SigningKeyshare {
                    owner_identifiers: signing_keyshare
                        .owner_identifiers
                        .into_iter()
                        .map(|id| {
                            Ok(Identifier::deserialize(
                                &hex::decode(&id)
                                    .map_err(|_| DepositServiceError::InvalidIdentifier)?,
                            )
                            .map_err(|_| DepositServiceError::InvalidIdentifier)?)
                        })
                        .collect::<Result<Vec<_>, DepositServiceError>>()?,
                    threshold: signing_keyshare.threshold,
                };

                Ok(TreeNode {
                    id: node.id,
                    tree_id: node.tree_id,
                    value: node.value,
                    parent_node_id: node.parent_node_id,
                    node_tx: deserialize(&node.node_tx)
                        .map_err(|_| DepositServiceError::InvalidTransaction)?,
                    refund_tx: deserialize(&node.refund_tx)
                        .map_err(|_| DepositServiceError::InvalidTransaction)?,
                    vout: node.vout,
                    verifying_public_key: PublicKey::from_slice(&node.verifying_public_key)
                        .map_err(|_| DepositServiceError::InvalidVerifyingKey)?,
                    owner_identity_public_key: PublicKey::from_slice(
                        &node.owner_identity_public_key,
                    )
                    .map_err(|_| DepositServiceError::InvalidPublicKey)?,
                    signing_keyshare,
                    status: node
                        .status
                        .parse()
                        .map_err(|_| DepositServiceError::UnknownStatus(node.status.clone()))?,
                })
            })
            .collect::<Result<Vec<_>, DepositServiceError>>()?;

        Ok(nodes)
    }

    pub async fn generate_deposit_address(
        &self,
        signing_public_key: PublicKey,
        leaf_id: String,
        is_static: bool,
    ) -> Result<DepositAddress, DepositServiceError> {
        let resp = self
            .client
            .generate_deposit_address(GenerateDepositAddressRequest {
                signing_public_key: signing_public_key.serialize().to_vec(),
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                network: self.spark_network() as i32,
                leaf_id: Some(leaf_id.clone()),
                is_static: Some(is_static),
            })
            .await?;

        let Some(deposit_address) = resp.deposit_address else {
            return Err(DepositServiceError::MissingDepositAddress);
        };

        let address =
            self.validate_deposit_address(deposit_address, signing_public_key, leaf_id)?;

        Ok(address)
    }

    pub async fn query_unused_deposit_addresses(
        &self,
    ) -> Result<Vec<DepositAddress>, DepositServiceError> {
        let resp = self
            .client
            .query_unused_deposit_addresses(
                spark_protos::spark::QueryUnusedDepositAddressesRequest {
                    identity_public_key: self.identity_public_key.serialize().to_vec(),
                    network: self.spark_network() as i32,
                },
            )
            .await?;

        let addresses = resp
            .deposit_addresses
            .into_iter()
            .map(|addr| {
                let address: Address<NetworkUnchecked> = addr
                    .deposit_address
                    .parse()
                    .map_err(|_| DepositServiceError::InvalidDepositAddress)?;

                Ok(DepositAddress {
                    address: address
                        .require_network(self.network.into())
                        .map_err(|_| DepositServiceError::InvalidDepositAddressNetwork)?,
                    // TODO: Is it possible addresses do not have a leaf_id?
                    leaf_id: addr.leaf_id.ok_or(DepositServiceError::MissingLeafId)?,
                    user_signing_public_key: PublicKey::from_slice(&addr.user_signing_public_key)
                        .map_err(|_| {
                        DepositServiceError::InvalidDepositAddressProof
                    })?,
                    verifying_public_key: PublicKey::from_slice(&addr.verifying_public_key)
                        .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?,
                })
            })
            .collect::<Result<Vec<_>, DepositServiceError>>()
            .map_err(|_| DepositServiceError::InvalidDepositAddress)?;

        Ok(addresses)
    }

    fn proof_of_possession_message_hash(
        &self,
        operator_public_key: &PublicKey,
        address: &Address,
    ) -> sha256::Hash {
        let mut msg = operator_public_key.serialize().to_vec();
        msg.extend_from_slice(&self.identity_public_key.serialize());
        msg.extend_from_slice(address.to_string().as_bytes());
        sha256::Hash::hash(&msg)
    }

    fn validate_deposit_address(
        &self,
        deposit_address: spark_protos::spark::Address,
        user_signing_public_key: PublicKey,
        leaf_id: String,
    ) -> Result<DepositAddress, DepositServiceError> {
        let address: Address<NetworkUnchecked> = deposit_address
            .address
            .parse()
            .map_err(|_| DepositServiceError::InvalidDepositAddress)?;
        let address = address
            .require_network(self.network.into())
            .map_err(|_| DepositServiceError::InvalidDepositAddressNetwork)?;

        let Some(proof) = deposit_address.deposit_address_proof else {
            return Err(DepositServiceError::MissingDepositAddressProof);
        };

        let verifying_public_key = PublicKey::from_slice(&deposit_address.verifying_key)
            .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?;

        let operator_public_key = self
            .bitcoin_service
            .subtract_public_keys(&verifying_public_key, &user_signing_public_key)
            .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?;
        let taproot_key = self
            .bitcoin_service
            .compute_taproot_key_no_script(&operator_public_key);
        let msg = self.proof_of_possession_message_hash(&operator_public_key, &address);
        let msg = Message::from_digest(msg.to_byte_array());
        let proof_of_possession_signature =
            schnorr::Signature::from_slice(&proof.proof_of_possession_signature)
                .map_err(|_| DepositServiceError::InvalidDepositAddressProof)?;
        if !self.bitcoin_service.is_valid_schnorr_signature(
            &proof_of_possession_signature,
            &msg,
            &taproot_key,
        ) {
            return Err(DepositServiceError::InvalidDepositAddressProof);
        }

        let address_hash = sha256::Hash::hash(address.to_string().as_bytes());
        let address_hash_message = Message::from_digest(address_hash.to_byte_array());
        for operator in self.operator_pool.get_signing_operators() {
            // TODO: rather than using hex::encode here, we should define our own type for the frost identifier, and use a hashmap with the identifier as key here.
            let Some(operator_sig) = proof
                .address_signatures
                .get(&hex::encode(&operator.identifier.serialize()))
            else {
                return Err(DepositServiceError::InvalidDepositAddressProof);
            };

            let Ok(operator_sig) = ecdsa::Signature::from_der(&operator_sig) else {
                return Err(DepositServiceError::InvalidDepositAddressProof);
            };

            if !self.bitcoin_service.is_valid_ecdsa_signature(
                &operator_sig,
                &address_hash_message,
                &operator.identity_public_key,
            ) {
                return Err(DepositServiceError::InvalidDepositAddressProof);
            }
        }

        Ok(DepositAddress {
            address,
            leaf_id,
            user_signing_public_key,
            verifying_public_key,
        })
    }
}

fn ephemeral_anchor_output() -> TxOut {
    TxOut {
        script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]), // Pay-to-anchor (P2A) ephemeral anchor output
        value: Amount::from_sat(0),
    }
}

fn marshal_frost_commitment(
    commitments: &SigningCommitments,
) -> Result<common::SigningCommitment, DepositServiceError> {
    let hiding = commitments.hiding().serialize().unwrap();
    let binding = commitments.binding().serialize().unwrap();

    Ok(common::SigningCommitment { hiding, binding })
}
