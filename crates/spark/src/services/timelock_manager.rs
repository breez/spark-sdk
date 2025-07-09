use bitcoin::absolute::LockTime;
use bitcoin::consensus::Encodable;
use bitcoin::secp256k1::PublicKey;
use bitcoin::transaction::Version;
use bitcoin::{OutPoint, Transaction, TxIn};
use std::sync::Arc;

use crate::Network;
use crate::bitcoin::sighash_from_tx;
use crate::core::{initial_sequence, next_sequence};
use crate::operator::rpc::common::SignatureIntent;
use crate::operator::rpc::spark::{
    ExtendLeafRequest, FinalizeNodeSignaturesRequest, NodeSignatures, SigningJob,
};
use crate::operator::rpc::{self as operator_rpc};
use crate::services::ClaimTransferConfig;
use crate::services::models::{
    map_public_keys, map_signature_shares, map_signing_nonce_commitments,
};
use crate::signer::{AggregateFrostRequest, PrivateKeySource, SignFrostRequest, Signer};
use crate::tree::TreeNode;
use crate::utils::refund::create_refund_tx;
use bitcoin::hashes::Hash;

use crate::services::ServiceError;

/// Utility for managing timelock operations
pub struct TimelockManager<S: Signer> {
    signer: S,
    network: Network,
    coordinator_client: Arc<operator_rpc::SparkRpcClient<S>>,
}

impl<S: Signer> TimelockManager<S> {
    pub fn new(
        signer: S,
        network: Network,
        coordinator_client: Arc<operator_rpc::SparkRpcClient<S>>,
    ) -> Self {
        Self {
            signer,
            network,
            coordinator_client,
        }
    }

    /// Post-processes claimed nodes (timelock operations)
    pub async fn post_process_claimed_nodes(
        &self,
        nodes: Vec<TreeNode>,
        config: &ClaimTransferConfig,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let mut processed_nodes = nodes;

        if config.should_refresh_timelocks {
            processed_nodes = self.check_refresh_timelock_nodes(&processed_nodes).await?;
        }

        if config.should_extend_timelocks {
            processed_nodes = self.check_extend_timelock_nodes(processed_nodes).await?;
        }

        Ok(processed_nodes)
    }

    /// Checks and refreshes timelock nodes if needed
    pub async fn check_refresh_timelock_nodes(
        &self,
        nodes: &[TreeNode],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: Implement timelock refresh logic
        // For now, return nodes unchanged
        Ok(nodes.to_vec())
    }

    /// Refreshes timelocks on a chain of connected nodes to prevent expiration.
    /// Updates sequence numbers on both node transactions and refund transactions
    /// in a coordinated manner across the entire chain.
    pub async fn refresh_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        todo!()
    }

    /// Checks and extends timelock nodes if needed
    pub async fn check_extend_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // if node needs to be extended, call extend_time_lock
        // TODO: implement
        // For now, return nodes unchanged
        Ok(nodes)
    }

    /// Extends the timelock on a single node by creating new node and refund transactions.
    /// Creates a new node transaction that spends the current node with an extended timelock,
    /// and a corresponding refund transaction. This is more comprehensive than refreshing
    /// as it creates entirely new transactions rather than just updating sequence numbers.
    pub async fn extend_time_lock(&self, node: &TreeNode) -> Result<Vec<TreeNode>, ServiceError> {
        let signing_key = PrivateKeySource::Derived(node.id.clone());
        let signing_public_key = self
            .signer
            .get_public_key_from_private_key_source(&signing_key)?;

        let refund_tx = node
            .refund_tx
            .clone()
            .ok_or(ServiceError::Generic("No refund tx".to_string()))?;

        let new_node_sequence = next_sequence(refund_tx.input[0].sequence).ok_or(
            ServiceError::Generic("Failed to get next sequence".to_string()),
        )?;

        let mut new_node_tx = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        new_node_tx.input.push(TxIn {
            previous_output: OutPoint {
                txid: node.node_tx.compute_txid(),
                vout: 0,
            },
            sequence: new_node_sequence,
            ..Default::default()
        });

        // TODO: js references applying a fee here, but is commented out. To do so, instead of cloning the output, we create a new one with the fee applied
        new_node_tx.output.push(node.node_tx.output[0].clone());

        let new_refund_tx = create_refund_tx(
            initial_sequence(),
            OutPoint {
                txid: new_node_tx.compute_txid(),
                vout: 0,
            },
            new_node_tx.output[0].value.to_sat(),
            &signing_public_key,
            self.network,
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;

        let node_sighash = sighash_from_tx(&new_node_tx, 0, &node.node_tx.output[0])?;
        let refund_sighash = sighash_from_tx(&new_refund_tx, 0, &new_node_tx.output[0])?;

        let new_node_signing_commitments = self.signer.generate_frost_signing_commitments().await?;
        let new_refund_signing_commitments =
            self.signer.generate_frost_signing_commitments().await?;

        let new_node_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: {
                let mut buf = Vec::new();
                new_node_tx
                    .consensus_encode(&mut buf)
                    .map_err(|e| ServiceError::BitcoinIOError(e))?;
                buf
            },
            signing_nonce_commitment: Some(new_node_signing_commitments.try_into()?),
        };

        let new_refund_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: {
                let mut buf = Vec::new();
                new_refund_tx
                    .consensus_encode(&mut buf)
                    .map_err(|e| ServiceError::BitcoinIOError(e))?;
                buf
            },
            signing_nonce_commitment: Some(new_refund_signing_commitments.try_into()?),
        };

        let response = self
            .coordinator_client
            .extend_leaf(ExtendLeafRequest {
                leaf_id: node.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                node_tx_signing_job: Some(new_node_signing_job),
                refund_tx_signing_job: Some(new_refund_signing_job),
            })
            .await?;

        let node_tx_signing_result =
            response
                .node_tx_signing_result
                .ok_or(ServiceError::Generic(
                    "Node tx signing result is none".to_string(),
                ))?;
        let refund_tx_signing_result =
            response
                .refund_tx_signing_result
                .ok_or(ServiceError::Generic(
                    "Refund tx signing result is none".to_string(),
                ))?;

        let new_node_tx_verifying_key =
            PublicKey::from_slice(&node_tx_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;
        let new_refund_tx_verifying_key =
            PublicKey::from_slice(&refund_tx_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

        let new_node_tx_signing_result =
            node_tx_signing_result
                .signing_result
                .ok_or(ServiceError::Generic(
                    "Node tx signing result is none".to_string(),
                ))?;
        let new_refund_tx_signing_result =
            refund_tx_signing_result
                .signing_result
                .ok_or(ServiceError::Generic(
                    "Refund tx signing result is none".to_string(),
                ))?;

        let new_node_statechain_commitments =
            map_signing_nonce_commitments(new_node_tx_signing_result.signing_nonce_commitments)?;
        let new_refund_statechain_commitments =
            map_signing_nonce_commitments(new_refund_tx_signing_result.signing_nonce_commitments)?;

        let new_node_statechain_signatures =
            map_signature_shares(new_node_tx_signing_result.signature_shares)?;
        let new_refund_statechain_signatures =
            map_signature_shares(new_refund_tx_signing_result.signature_shares)?;

        let new_node_statechain_public_keys =
            map_public_keys(new_node_tx_signing_result.public_keys)?;
        let new_refund_statechain_public_keys =
            map_public_keys(new_refund_tx_signing_result.public_keys)?;

        // sign node and refund txs
        let node_user_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: node_sighash.as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_key,
                verifying_key: &new_node_tx_verifying_key,
                self_commitment: &new_node_signing_commitments,
                statechain_commitments: new_node_statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let refund_user_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: refund_sighash.as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_key,
                verifying_key: &new_refund_tx_verifying_key,
                self_commitment: &new_refund_signing_commitments,
                statechain_commitments: new_refund_statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let node_signature = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: node_sighash.as_byte_array(),
                statechain_signatures: new_node_statechain_signatures,
                statechain_public_keys: new_node_statechain_public_keys,
                verifying_key: &new_node_tx_verifying_key,
                statechain_commitments: new_node_statechain_commitments,
                self_commitment: &new_node_signing_commitments,
                public_key: &signing_public_key,
                self_signature: &node_user_signature,
                adaptor_public_key: None,
            })
            .await?;

        let refund_signature = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: refund_sighash.as_byte_array(),
                statechain_signatures: new_refund_statechain_signatures,
                statechain_public_keys: new_refund_statechain_public_keys,
                verifying_key: &new_refund_tx_verifying_key,
                statechain_commitments: new_refund_statechain_commitments,
                self_commitment: &new_refund_signing_commitments,
                public_key: &signing_public_key,
                self_signature: &refund_user_signature,
                adaptor_public_key: None,
            })
            .await?;

        let nodes = self
            .coordinator_client
            .finalize_node_signatures(FinalizeNodeSignaturesRequest {
                intent: SignatureIntent::Extend.into(),
                node_signatures: vec![NodeSignatures {
                    node_id: response.leaf_id,
                    node_tx_signature: node_signature.serialize()?.to_vec(),
                    refund_tx_signature: refund_signature.serialize()?.to_vec(),
                }],
            })
            .await?
            .nodes;

        Ok(nodes
            .into_iter()
            .map(|n| n.try_into())
            .collect::<Result<Vec<TreeNode>, _>>()?)
    }
}
