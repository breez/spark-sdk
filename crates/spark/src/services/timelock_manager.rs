use std::{collections::HashMap, str::FromStr as _, sync::Arc};

use bitcoin::{OutPoint, hashes::Hash, secp256k1::PublicKey};
use tracing::trace;

use crate::{
    Network,
    bitcoin::sighash_from_tx,
    core::{initial_sequence, next_sequence},
    operator::{
        OperatorPool,
        rpc::{
            QueryAllNodesRequest,
            common::SignatureIntent,
            spark::{
                ExtendLeafRequest, FinalizeNodeSignaturesRequest, NodeSignatures,
                RefreshTimelockRequest, SigningJob, TreeNodeIds, query_nodes_request::Source,
            },
        },
    },
    services::{
        LeafKeyTweak, ServiceError, TransferService, map_public_keys, map_signature_shares,
        map_signing_nonce_commitments,
    },
    signer::{AggregateFrostRequest, PrivateKeySource, SignFrostRequest, Signer},
    tree::{TreeNode, TreeNodeId},
    utils::transactions::{create_node_tx, create_refund_tx},
};

pub struct TimelockManager<S> {
    signer: S,
    network: Network,
    operator_pool: Arc<OperatorPool<S>>,
    transfer_service: Arc<TransferService<S>>,
}

impl<S: Signer> TimelockManager<S> {
    pub fn new(
        signer: S,
        network: Network,
        operator_pool: Arc<OperatorPool<S>>,
        transfer_service: Arc<TransferService<S>>,
    ) -> Self {
        Self {
            signer,
            network,
            operator_pool,
            transfer_service,
        }
    }

    pub async fn check_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Checking timelock nodes: {:?}", nodes);
        let nodes = self.check_refresh_timelock_nodes(nodes).await?;
        let nodes = self.check_extend_timelock_nodes(nodes).await?;
        Ok(nodes)
    }

    /// Checks and refreshes timelock nodes if needed
    async fn check_refresh_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Checking refresh timelock nodes: {:?}", nodes);
        let mut nodes_to_refresh = Vec::new();
        let mut ready_nodes = Vec::new();

        for node in nodes {
            if node.needs_timelock_refresh()? {
                nodes_to_refresh.push(node);
            } else {
                ready_nodes.push(node);
            }
        }

        if nodes_to_refresh.is_empty() {
            return Ok(ready_nodes);
        }

        // Get the parent nodes
        let query_nodes_response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_nodes_all_pages(QueryAllNodesRequest {
                include_parents: true,
                network: self.network.into(),
                source: Some(Source::NodeIds(TreeNodeIds {
                    node_ids: nodes_to_refresh.iter().map(|n| n.id.to_string()).collect(),
                })),
            })
            .await?;

        let mut node_ids_to_nodes_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();
        for node in query_nodes_response.nodes.values() {
            node_ids_to_nodes_map.insert(
                TreeNodeId::from_str(&node.id).map_err(|e| ServiceError::ValidationError(e))?,
                node.clone().try_into()?,
            );
        }

        let mut refresh_tasks = Vec::new();
        for node in nodes_to_refresh {
            let parent_node = node_ids_to_nodes_map
                .get(
                    &node
                        .parent_node_id
                        .clone()
                        .ok_or(ServiceError::Generic("Node has no parent node".to_string()))?,
                )
                .ok_or(ServiceError::Generic(
                    "Parent node not found in queried nodes".to_string(),
                ))?;

            refresh_tasks.push(self.refresh_timelock_node(node, parent_node));
        }

        let refreshed_nodes = futures::future::try_join_all(refresh_tasks).await?;
        ready_nodes.extend(refreshed_nodes);

        // TODO: update local tree to avoid having to re-fetch after this

        Ok(ready_nodes)
    }

    /// Refreshes the timelock on a single node by decrementing the timelock on the node tx
    /// and rebuilding the refund tx with the initial timelock.
    ///
    /// Should be done when the refund tx timelock is about to expire.
    async fn refresh_timelock_node(
        &self,
        node: TreeNode,
        parent_node: &TreeNode,
    ) -> Result<TreeNode, ServiceError> {
        trace!("Refreshing timelock node: {:?}", node.id);
        let signing_key = PrivateKeySource::Derived(node.id.clone());
        let signing_public_key = self
            .signer
            .get_public_key_from_private_key_source(&signing_key)?;

        let node_tx_input = node.node_tx.input[0].clone();

        let new_node_tx = create_node_tx(
            next_sequence(node_tx_input.sequence).ok_or(ServiceError::Generic(
                "Failed to get next sequence".to_string(),
            ))?,
            node_tx_input.previous_output,
            node.node_tx.output[0].value,
            node.node_tx.output[0].script_pubkey.clone(),
        );

        let refund_tx = node
            .refund_tx
            .clone()
            .ok_or(ServiceError::Generic("No refund tx".to_string()))?;

        let new_refund_tx = create_refund_tx(
            initial_sequence(),
            OutPoint {
                txid: new_node_tx.compute_txid(),
                vout: 0,
            },
            refund_tx.output[0].value.to_sat(),
            &signing_public_key,
            self.network,
        );

        let new_node_signing_commitments = self.signer.generate_frost_signing_commitments().await?;
        let new_refund_signing_commitments =
            self.signer.generate_frost_signing_commitments().await?;

        let signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&new_node_tx),
            signing_nonce_commitment: Some(new_node_signing_commitments.try_into()?),
        };

        let refund_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&new_refund_tx),
            signing_nonce_commitment: Some(new_refund_signing_commitments.try_into()?),
        };

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .refresh_timelock(RefreshTimelockRequest {
                leaf_id: node.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                signing_jobs: vec![signing_job, refund_signing_job],
            })
            .await?;

        if response.signing_results.len() != 2 {
            return Err(ServiceError::Generic(
                "Expected 2 signing results".to_string(),
            ));
        }

        let node_tx_signing_result = &response.signing_results[0];
        let refund_tx_signing_result = &response.signing_results[1];

        let node_sighash = sighash_from_tx(&new_node_tx, 0, &parent_node.node_tx.output[0])?;
        let refund_sighash = sighash_from_tx(&new_refund_tx, 0, &new_node_tx.output[0])?;

        let new_node_tx_verifying_key =
            PublicKey::from_slice(&node_tx_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;
        let new_refund_tx_verifying_key =
            PublicKey::from_slice(&refund_tx_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

        let new_node_tx_signing_result =
            node_tx_signing_result
                .signing_result
                .as_ref()
                .ok_or(ServiceError::Generic(
                    "Node tx signing result is none".to_string(),
                ))?;
        let new_refund_tx_signing_result =
            refund_tx_signing_result
                .signing_result
                .as_ref()
                .ok_or(ServiceError::Generic(
                    "Refund tx signing result is none".to_string(),
                ))?;

        let new_node_statechain_commitments =
            map_signing_nonce_commitments(&new_node_tx_signing_result.signing_nonce_commitments)?;
        let new_refund_statechain_commitments =
            map_signing_nonce_commitments(&new_refund_tx_signing_result.signing_nonce_commitments)?;

        let new_node_statechain_signatures =
            map_signature_shares(&new_node_tx_signing_result.signature_shares)?;
        let new_refund_statechain_signatures =
            map_signature_shares(&new_refund_tx_signing_result.signature_shares)?;

        let new_node_statechain_public_keys =
            map_public_keys(&new_node_tx_signing_result.public_keys)?;
        let new_refund_statechain_public_keys =
            map_public_keys(&new_refund_tx_signing_result.public_keys)?;

        let user_node_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: node_sighash.as_raw_hash().as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_key,
                verifying_key: &new_node_tx_verifying_key,
                self_commitment: &new_node_signing_commitments,
                statechain_commitments: new_node_statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let node_signature = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: node_sighash.as_raw_hash().as_byte_array(),
                statechain_signatures: new_node_statechain_signatures,
                statechain_public_keys: new_node_statechain_public_keys,
                verifying_key: &new_node_tx_verifying_key,
                statechain_commitments: new_node_statechain_commitments,
                self_commitment: &new_node_signing_commitments,
                public_key: &signing_public_key,
                self_signature: &user_node_signature,
                adaptor_public_key: None,
            })
            .await?;

        let user_refund_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: refund_sighash.as_raw_hash().as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_key,
                verifying_key: &new_refund_tx_verifying_key,
                self_commitment: &new_refund_signing_commitments,
                statechain_commitments: new_refund_statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let refund_signature = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: refund_sighash.as_raw_hash().as_byte_array(),
                statechain_signatures: new_refund_statechain_signatures,
                statechain_public_keys: new_refund_statechain_public_keys,
                verifying_key: &new_refund_tx_verifying_key,
                statechain_commitments: new_refund_statechain_commitments,
                self_commitment: &new_refund_signing_commitments,
                public_key: &signing_public_key,
                self_signature: &user_refund_signature,
                adaptor_public_key: None,
            })
            .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .finalize_node_signatures(FinalizeNodeSignaturesRequest {
                intent: SignatureIntent::Refresh.into(),
                node_signatures: vec![NodeSignatures {
                    node_id: node.id.to_string(),
                    node_tx_signature: node_signature.serialize()?.to_vec(),
                    refund_tx_signature: refund_signature.serialize()?.to_vec(),
                }],
            })
            .await?;

        if response.nodes.len() != 1 {
            return Err(ServiceError::Generic(
                "Expected 1 node in response".to_string(),
            ));
        }

        Ok(response.nodes[0].clone().try_into()?)
    }

    /// Checks and extends timelock nodes if needed
    async fn check_extend_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Checking extend timelock nodes: {:?}", nodes);
        let mut nodes_to_extend = Vec::new();
        let mut ready_nodes = Vec::new();

        for node in nodes {
            if node.needs_timelock_extension()? {
                nodes_to_extend.push(node);
            } else {
                ready_nodes.push(node);
            }
        }

        if nodes_to_extend.is_empty() {
            return Ok(ready_nodes);
        }

        let mut extend_tasks = Vec::new();
        for node in nodes_to_extend {
            extend_tasks.push(async move {
                let extended_nodes = self.extend_time_lock(&node).await?;
                let our_extended_nodes = self.transfer_leaves_to_self(extended_nodes).await?;
                Ok::<Vec<TreeNode>, ServiceError>(our_extended_nodes)
            });
        }

        let extended_nodes = futures::future::try_join_all(extend_tasks).await?;
        ready_nodes.extend(extended_nodes.into_iter().flatten().collect::<Vec<_>>());

        // TODO: update local tree to avoid having to re-fetch after this

        Ok(ready_nodes)
    }

    /// Extends the timelock of a node by creating a new child node (this having the initial timelock on both node and refund txs).
    ///
    /// Should be done when the node tx timelock is about to expire.
    pub(crate) async fn extend_time_lock(
        &self,
        node: &TreeNode,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Extending timelock node: {:?}", node.id);
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

        let new_node_tx = create_node_tx(
            new_node_sequence,
            OutPoint {
                txid: node.node_tx.compute_txid(),
                vout: 0,
            },
            node.node_tx.output[0].value,
            node.node_tx.output[0].script_pubkey.clone(),
        );

        let new_refund_tx = create_refund_tx(
            initial_sequence(),
            OutPoint {
                txid: new_node_tx.compute_txid(),
                vout: 0,
            },
            new_node_tx.output[0].value.to_sat(),
            &signing_public_key,
            self.network,
        );

        let node_sighash = sighash_from_tx(&new_node_tx, 0, &node.node_tx.output[0])?;
        let refund_sighash = sighash_from_tx(&new_refund_tx, 0, &new_node_tx.output[0])?;

        let new_node_signing_commitments = self.signer.generate_frost_signing_commitments().await?;
        let new_refund_signing_commitments =
            self.signer.generate_frost_signing_commitments().await?;

        let new_node_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&new_node_tx),
            signing_nonce_commitment: Some(new_node_signing_commitments.try_into()?),
        };

        let new_refund_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&new_refund_tx),
            signing_nonce_commitment: Some(new_refund_signing_commitments.try_into()?),
        };

        let response = self
            .operator_pool
            .get_coordinator()
            .client
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
            map_signing_nonce_commitments(&new_node_tx_signing_result.signing_nonce_commitments)?;
        let new_refund_statechain_commitments =
            map_signing_nonce_commitments(&new_refund_tx_signing_result.signing_nonce_commitments)?;

        let new_node_statechain_signatures =
            map_signature_shares(&new_node_tx_signing_result.signature_shares)?;
        let new_refund_statechain_signatures =
            map_signature_shares(&new_refund_tx_signing_result.signature_shares)?;

        let new_node_statechain_public_keys =
            map_public_keys(&new_node_tx_signing_result.public_keys)?;
        let new_refund_statechain_public_keys =
            map_public_keys(&new_refund_tx_signing_result.public_keys)?;

        // sign node and refund txs
        let node_user_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: node_sighash.as_raw_hash().as_byte_array(),
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
                message: refund_sighash.as_raw_hash().as_byte_array(),
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
                message: node_sighash.as_raw_hash().as_byte_array(),
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
                message: refund_sighash.as_raw_hash().as_byte_array(),
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
            .operator_pool
            .get_coordinator()
            .client
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

        nodes
            .into_iter()
            .map(|n| n.try_into())
            .collect::<Result<Vec<TreeNode>, _>>()
    }

    pub(crate) async fn transfer_leaves_to_self(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let leaf_key_tweaks = leaves
            .iter()
            .map(|leaf| {
                let current_signing_key =
                    PrivateKeySource::Derived(leaf.parent_node_id.clone().ok_or(
                        ServiceError::Generic("Leaf has no parent node id".to_string()),
                    )?);
                let ephemeral_key = self.signer.generate_random_key()?;

                Ok(LeafKeyTweak {
                    node: leaf.clone(),
                    signing_key: current_signing_key,
                    new_signing_key: ephemeral_key,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let transfer = self
            .transfer_service
            .send_transfer_with_key_tweaks(
                &leaf_key_tweaks,
                &self.signer.get_identity_public_key()?,
            )
            .await?;

        let pending_transfer = self
            .transfer_service
            .query_transfer(&transfer.id)
            .await?
            .ok_or(ServiceError::Generic(
                "Pending transfer not found".to_string(),
            ))?;

        let resulting_nodes = self
            .transfer_service
            .claim_transfer(&pending_transfer, None)
            .await?;

        Ok(resulting_nodes)
    }
}
