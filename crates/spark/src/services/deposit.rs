use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut,
    absolute::LockTime,
    address::NetworkUnchecked,
    consensus::{deserialize, serialize},
    hashes::{Hash, sha256},
    params::Params,
    secp256k1::{Message, PublicKey, ecdsa, schnorr},
    transaction::Version,
};
use frost_secp256k1_tr::Identifier;

use crate::{
    Network,
    bitcoin::{BitcoinService, sighash_from_tx},
    core::initial_sequence,
    operator::{OperatorPool, rpc::SparkRpcClient},
    signer::Signer,
    tree::{SigningKeyshare, TreeNode, TreeNodeId},
};
use spark_protos::{
    common::{self, SignatureIntent},
    spark::{
        self, FinalizeNodeSignaturesRequest, GenerateDepositAddressRequest, NodeSignatures,
        SigningJob, StartDepositTreeCreationRequest,
    },
};

use super::{
    ServiceError,
    models::{
        map_public_keys, map_signature_shares, map_signing_nonce_commitments,
        marshal_frost_commitment,
    },
};
pub struct DepositService<S>
where
    S: Signer,
{
    bitcoin_service: BitcoinService,
    client: Arc<SparkRpcClient<S>>,
    identity_public_key: PublicKey,
    network: Network,
    operator_pool: OperatorPool,
    signer: S,
}

pub struct DepositAddress {
    pub address: Address,
    pub leaf_id: TreeNodeId,
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
        client: Arc<SparkRpcClient<S>>,
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

    pub async fn claim_deposit(
        &self,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: Ensure all inputs are segwit inputs, so this tx is not malleable. Normally the tx should be already confirmed, but perhaps we get in trouble with a reorg?

        let params: Params = self.network.into();

        let output: &TxOut = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;
        let address = Address::from_script(&output.script_pubkey, params)
            .map_err(|_| ServiceError::NotADepositOutput)?;
        let deposit_address = self
            .get_unused_deposit_address(&address)
            .await?
            .ok_or(ServiceError::DepositAddressUsed)?;
        let signing_public_key = self
            .signer
            .get_public_key_for_node(&deposit_address.leaf_id)?;
        let nodes = self
            .create_tree_root(
                &signing_public_key,
                &deposit_address.verifying_public_key,
                deposit_tx,
                vout,
            )
            .await?;
        Ok(nodes)
    }

    async fn create_tree_root(
        &self,
        signing_public_key: &PublicKey,
        verifying_public_key: &PublicKey,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let deposit_txid = deposit_tx.compute_txid();
        let deposit_output = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;
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

        let root_node_signature_shares = tree_resp
            .root_node_signature_shares
            .ok_or(ServiceError::MissingTreeSignatures)?;
        let node_tx_signing_result = root_node_signature_shares
            .node_tx_signing_result
            .ok_or(ServiceError::MissingTreeSignatures)?;
        let refund_tx_signing_result = root_node_signature_shares
            .refund_tx_signing_result
            .ok_or(ServiceError::MissingTreeSignatures)?;

        if node_tx_signing_result.signing_nonce_commitments.is_empty() {
            return Err(ServiceError::MissingTreeSignatures);
        }

        let node_tx_signing_nonce_commitments =
            map_signing_nonce_commitments(node_tx_signing_result.signing_nonce_commitments)?;
        let node_tx_signature_shares =
            map_signature_shares(node_tx_signing_result.signature_shares)?;
        let node_tx_statechain_public_keys = map_public_keys(node_tx_signing_result.public_keys)?;

        if refund_tx_signing_result
            .signing_nonce_commitments
            .is_empty()
        {
            return Err(ServiceError::MissingTreeSignatures);
        }

        let refund_tx_signing_nonce_commitments =
            map_signing_nonce_commitments(refund_tx_signing_result.signing_nonce_commitments)?;
        let refund_tx_signature_shares =
            map_signature_shares(refund_tx_signing_result.signature_shares)?;
        let refund_tx_statechain_public_keys =
            map_public_keys(refund_tx_signing_result.public_keys)?;

        let tree_resp_verifying_key =
            PublicKey::from_slice(&root_node_signature_shares.verifying_key)
                .map_err(|_| ServiceError::InvalidVerifyingKey)?;

        if &tree_resp_verifying_key != verifying_public_key {
            return Err(ServiceError::InvalidVerifyingKey);
        }

        let root_sig = self
            .signer
            .sign_frost(
                &root_tx_sighash.to_byte_array(),
                signing_public_key,
                signing_public_key,
                verifying_public_key,
                &root_nonce_commitment,
                node_tx_signing_nonce_commitments.clone(),
                None,
            )
            .await?;
        let refund_sig = self
            .signer
            .sign_frost(
                &refund_tx_sighash.to_byte_array(),
                signing_public_key,
                signing_public_key,
                verifying_public_key,
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
                verifying_public_key,
                node_tx_signing_nonce_commitments,
                &root_nonce_commitment,
                signing_public_key,
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
                verifying_public_key,
                refund_tx_signing_nonce_commitments,
                &refund_nonce_commitment,
                signing_public_key,
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
                        .map_err(|_| ServiceError::InvalidSignatureShare)?
                        .to_vec(),
                    refund_tx_signature: refund_aggregate
                        .serialize()
                        .map_err(|_| ServiceError::InvalidSignatureShare)?
                        .to_vec(),
                }],
            })
            .await?;

        // TODO: Verify the returned tx signatures

        let nodes = finalize_resp
            .nodes
            .into_iter()
            .map(|node| {
                let signing_keyshare = node
                    .signing_keyshare
                    .ok_or(ServiceError::MissingSigningKeyshare)?;
                let signing_keyshare = SigningKeyshare {
                    owner_identifiers: signing_keyshare
                        .owner_identifiers
                        .into_iter()
                        .map(|id| {
                            Identifier::deserialize(
                                &hex::decode(&id).map_err(|_| ServiceError::InvalidIdentifier)?,
                            )
                            .map_err(|_| ServiceError::InvalidIdentifier)
                        })
                        .collect::<Result<Vec<_>, ServiceError>>()?,
                    threshold: signing_keyshare.threshold,
                };

                Ok(TreeNode {
                    id: node
                        .id
                        .parse()
                        .map_err(|_| ServiceError::InvalidNodeId(node.id))?,
                    tree_id: node.tree_id,
                    value: node.value,
                    parent_node_id: match node.parent_node_id {
                        Some(id) => Some(id.parse().map_err(|_| ServiceError::InvalidNodeId(id))?),
                        None => None,
                    },
                    node_tx: deserialize(&node.node_tx)
                        .map_err(|_| ServiceError::InvalidTransaction)?,
                    refund_tx: deserialize(&node.refund_tx)
                        .map_err(|_| ServiceError::InvalidTransaction)?,
                    vout: node.vout,
                    verifying_public_key: PublicKey::from_slice(&node.verifying_public_key)
                        .map_err(|_| ServiceError::InvalidVerifyingKey)?,
                    owner_identity_public_key: PublicKey::from_slice(
                        &node.owner_identity_public_key,
                    )
                    .map_err(|_| ServiceError::InvalidPublicKey)?,
                    signing_keyshare,
                    status: node
                        .status
                        .parse()
                        .map_err(|_| ServiceError::UnknownStatus(node.status.clone()))?,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        Ok(nodes)
    }

    pub async fn generate_deposit_address(
        &self,
        signing_public_key: PublicKey,
        leaf_id: &TreeNodeId,
        is_static: bool,
    ) -> Result<DepositAddress, ServiceError> {
        let resp = self
            .client
            .generate_deposit_address(GenerateDepositAddressRequest {
                signing_public_key: signing_public_key.serialize().to_vec(),
                identity_public_key: self.identity_public_key.serialize().to_vec(),
                network: self.spark_network() as i32,
                leaf_id: Some(leaf_id.to_string()),
                is_static: Some(is_static),
            })
            .await?;

        let Some(deposit_address) = resp.deposit_address else {
            return Err(ServiceError::MissingDepositAddress);
        };

        let address =
            self.validate_deposit_address(deposit_address, signing_public_key, leaf_id)?;

        Ok(address)
    }

    pub async fn get_unused_deposit_address(
        &self,
        address: &Address,
    ) -> Result<Option<DepositAddress>, ServiceError> {
        // TODO: unused deposit addresses could be cached in the wallet, so they don't have to be queried from the server every time.
        Ok(self
            .query_unused_deposit_addresses()
            .await?
            .into_iter()
            .find(|d| &d.address == address))
    }

    pub async fn query_unused_deposit_addresses(
        &self,
    ) -> Result<Vec<DepositAddress>, ServiceError> {
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
                    .map_err(|_| ServiceError::InvalidDepositAddress)?;

                Ok(DepositAddress {
                    address: address
                        .require_network(self.network.into())
                        .map_err(|_| ServiceError::InvalidDepositAddressNetwork)?,
                    // TODO: Is it possible addresses do not have a leaf_id?
                    leaf_id: addr
                        .leaf_id
                        .ok_or(ServiceError::MissingLeafId)?
                        .parse()
                        .map_err(ServiceError::InvalidNodeId)?,
                    user_signing_public_key: PublicKey::from_slice(&addr.user_signing_public_key)
                        .map_err(|_| {
                        ServiceError::InvalidDepositAddressProof
                    })?,
                    verifying_public_key: PublicKey::from_slice(&addr.verifying_public_key)
                        .map_err(|_| ServiceError::InvalidDepositAddressProof)?,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;

        Ok(addresses)
    }

    fn proof_of_possession_message_hash(
        &self,
        operator_public_key: &PublicKey,
        address: &Address,
    ) -> sha256::Hash {
        let mut msg = self.identity_public_key.serialize().to_vec();
        msg.extend_from_slice(&operator_public_key.serialize());
        msg.extend_from_slice(address.to_string().as_bytes());
        sha256::Hash::hash(&msg)
    }

    fn validate_deposit_address(
        &self,
        deposit_address: spark_protos::spark::Address,
        user_signing_public_key: PublicKey,
        leaf_id: &TreeNodeId,
    ) -> Result<DepositAddress, ServiceError> {
        let address: Address<NetworkUnchecked> = deposit_address
            .address
            .parse()
            .map_err(|_| ServiceError::InvalidDepositAddress)?;
        let address = address
            .require_network(self.network.into())
            .map_err(|_| ServiceError::InvalidDepositAddressNetwork)?;

        let Some(proof) = deposit_address.deposit_address_proof else {
            return Err(ServiceError::MissingDepositAddressProof);
        };

        let verifying_public_key = PublicKey::from_slice(&deposit_address.verifying_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;

        let operator_public_key = self
            .bitcoin_service
            .subtract_public_keys(&verifying_public_key, &user_signing_public_key)
            .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
        let taproot_key = self
            .bitcoin_service
            .compute_taproot_key_no_script(&operator_public_key);

        // Note this is not a proof of possession really, but rather a commitment by the server that they associate the address with the user's identity.
        let msg = self.proof_of_possession_message_hash(&operator_public_key, &address);
        let msg = Message::from_digest(msg.to_byte_array());
        let proof_of_possession_signature =
            schnorr::Signature::from_slice(&proof.proof_of_possession_signature)
                .map_err(|_| ServiceError::InvalidDepositAddressProof)?;
        if !self.bitcoin_service.is_valid_schnorr_signature(
            &proof_of_possession_signature,
            &msg,
            &taproot_key,
        ) {
            return Err(ServiceError::InvalidDepositAddressProof);
        }

        let address_hash = sha256::Hash::hash(address.to_string().as_bytes());
        let address_hash_message = Message::from_digest(address_hash.to_byte_array());
        for operator in self.operator_pool.get_signing_operators() {
            // TODO: rather than using hex::encode here, we should define our own type for the frost identifier, and use a hashmap with the identifier as key here.
            let Some(operator_sig) = proof
                .address_signatures
                .get(&hex::encode(operator.identifier.serialize()))
            else {
                return Err(ServiceError::InvalidDepositAddressProof);
            };

            let Ok(operator_sig) = ecdsa::Signature::from_der(operator_sig) else {
                return Err(ServiceError::InvalidDepositAddressProof);
            };

            if !self.bitcoin_service.is_valid_ecdsa_signature(
                &operator_sig,
                &address_hash_message,
                &operator.identity_public_key,
            ) {
                return Err(ServiceError::InvalidDepositAddressProof);
            }
        }

        Ok(DepositAddress {
            address,
            leaf_id: leaf_id.clone(),
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
