use std::{collections::HashMap, str::FromStr as _, sync::Arc};

use bitcoin::{OutPoint, secp256k1::PublicKey};
use tracing::{trace, warn};

use crate::{
    Network,
    core::{initial_cpfp_sequence, initial_direct_sequence, next_sequence},
    operator::{
        OperatorPool,
        rpc::{
            QueryAllNodesRequest,
            common::SignatureIntent,
            spark::{
                ExtendLeafRequest, FinalizeNodeSignaturesRequest, NodeSignatures,
                RefreshTimelockRequest, TreeNodeIds, query_nodes_request::Source,
            },
        },
    },
    services::{
        ExtendLeafSigningResult, ServiceError, SigningJob, SigningJobTxType, TransferService,
    },
    signer::{PrivateKeySource, Signer},
    tree::{TreeNode, TreeNodeId},
    utils::{
        frost::{SignAggregateFrostParams, sign_aggregate_frost},
        transactions::{NodeTransactions, RefundTransactions, create_node_txs, create_refund_txs},
    },
};

pub struct TimelockManager {
    signer: Arc<dyn Signer>,
    network: Network,
    operator_pool: Arc<OperatorPool>,
    transfer_service: Arc<TransferService>,
}

impl TimelockManager {
    pub fn new(
        signer: Arc<dyn Signer>,
        network: Network,
        operator_pool: Arc<OperatorPool>,
        transfer_service: Arc<TransferService>,
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
        match self.check_extend_timelock_nodes(nodes.clone()).await {
            Ok(nodes) => Ok(nodes),
            Err(e) => {
                warn!("Error checking extend timelock nodes: {:?}", e);
                Err(ServiceError::PartialCheckTimelockError(nodes))
            }
        }
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
                TreeNodeId::from_str(&node.id).map_err(ServiceError::ValidationError)?,
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

        Ok(ready_nodes)
    }

    /// Refreshes a timelock node by creating new transactions with decreased timelocks.
    ///
    /// This function decreases the timelock values in the node transaction to enable earlier
    /// spending. It's part of the Spark protocol's safety mechanism to ensure funds can be
    /// recovered if channel operations fail or are delayed.
    ///
    /// Transaction Relationship Structure:
    /// ```ignore
    ///                           +----------------+
    ///                           | Parent Node TX |
    ///                           +-------+--------+
    ///                                   |
    ///                     +-------------+--------------+
    ///                     |                            |
    ///           +---------v----------+       +---------v----------+
    ///           | CPFP Node TX       |       | Direct Node TX     |
    ///           | (decreased seq)    |       | (decreased seq)    |
    ///           | (anchor, no fee)   |       | (no anchor, fee)   |
    ///           +---------+----------+       +---------+----------+
    ///                     |                            |
    ///      +--------------+-------------+              +----------+
    ///      |                            |                         |
    /// +----v-------------+      +-------v----------+       +------v-----------+
    /// | CPFP Refund TX   |      | Direct From CPFP |       | Direct Refund TX |
    /// | (anchor, no fee) |      | Refund TX        |       | (no anchor, fee) |
    /// |                  |      | (no anchor, fee) |       |                  |
    /// +------------------+      +------------------+       +------------------+
    /// ```
    ///
    /// The function:
    /// 1. Calculates new, decreased sequence numbers using the `next_sequence` function
    /// 2. Creates new node transactions (CPFP and Direct) with the decreased timelocks
    /// 3. Creates new refund transactions for the newly created node transactions
    /// 4. Sets up signing commitments for all transactions
    /// 5. Signs all transactions using FROST threshold signatures
    /// 6. Finalizes the signatures with operators
    ///
    /// # Arguments
    ///
    /// * `node` - The node to refresh
    /// * `parent_node` - The parent of the node to be refreshed
    ///
    /// # Returns
    ///
    /// * `Ok(TreeNode)` - The refreshed tree node with updated transactions
    /// * `Err(ServiceError)` - If any part of the refresh process fails
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

        let parent_node_tx = &parent_node.node_tx;
        let parent_node_tx_out = &parent_node_tx.output[0];

        let node_tx = &node.node_tx;
        let node_outpoint = node_tx.input[0].previous_output;
        let node_tx_out = &node_tx.output[0];

        let direct_tx = node.direct_tx;
        let direct_outpoint = direct_tx.as_ref().map(|tx| tx.input[0].previous_output);

        let old_sequence = node_tx.input[0].sequence;
        let (cpfp_sequence, direct_sequence) = next_sequence(old_sequence).ok_or(
            ServiceError::Generic("Failed to get next sequence".to_string()),
        )?;

        let NodeTransactions {
            cpfp_tx: cpfp_node_tx,
            direct_tx: direct_node_tx,
        } = create_node_txs(
            cpfp_sequence,
            direct_sequence,
            node_outpoint,
            direct_outpoint,
            parent_node_tx_out.value,
            parent_node_tx_out.script_pubkey.clone(),
            true,
        );

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: direct_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = create_refund_txs(
            initial_cpfp_sequence(),
            initial_direct_sequence(),
            OutPoint {
                txid: cpfp_node_tx.compute_txid(),
                vout: 0,
            },
            direct_node_tx.as_ref().map(|tx| OutPoint {
                txid: tx.compute_txid(),
                vout: 0,
            }),
            node_tx_out.value.to_sat(),
            &signing_public_key,
            self.network,
        );

        let mut signing_jobs = Vec::new();

        signing_jobs.push(SigningJob {
            tx_type: SigningJobTxType::CpfpNode,
            tx: cpfp_node_tx.clone(),
            parent_tx_out: parent_node_tx_out.clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_frost_signing_commitments().await?,
        });
        signing_jobs.push(SigningJob {
            tx_type: SigningJobTxType::CpfpRefund,
            tx: cpfp_refund_tx,
            parent_tx_out: cpfp_node_tx.output[0].clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_frost_signing_commitments().await?,
        });

        if let Some(direct_node_tx) = &direct_node_tx {
            signing_jobs.push(SigningJob {
                tx_type: SigningJobTxType::DirectNode,
                tx: direct_node_tx.clone(),
                parent_tx_out: parent_node_tx_out.clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_frost_signing_commitments().await?,
            });
        }
        if let (Some(direct_refund_tx), Some(direct_node_tx)) = (direct_refund_tx, &direct_node_tx)
        {
            signing_jobs.push(SigningJob {
                tx_type: SigningJobTxType::DirectRefund,
                tx: direct_refund_tx.clone(),
                parent_tx_out: direct_node_tx.output[0].clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_frost_signing_commitments().await?,
            });
        }
        if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            signing_jobs.push(SigningJob {
                tx_type: SigningJobTxType::DirectFromCpfpRefund,
                tx: direct_from_cpfp_refund_tx,
                parent_tx_out: cpfp_node_tx.output[0].clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_frost_signing_commitments().await?,
            });
        }

        let signing_jobs_count = signing_jobs.len();
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .refresh_timelock_v2(RefreshTimelockRequest {
                leaf_id: node.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                signing_jobs: signing_jobs
                    .iter()
                    .map(|job| job.try_into())
                    .collect::<Result<Vec<_>, _>>()?,
            })
            .await?;

        if response.signing_results.len() != signing_jobs_count {
            return Err(ServiceError::Generic(format!(
                "Expected {signing_jobs_count} signing results"
            )));
        }

        let mut node_signatures = NodeSignatures {
            node_id: node.id.to_string(),
            ..Default::default()
        };

        for (i, signing_result) in response.signing_results.iter().enumerate() {
            let signing_job = &signing_jobs[i];
            trace!("Processing signing job: {:?}", signing_job.tx_type);

            let verifying_key = PublicKey::from_slice(&signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

            let signing_result = signing_result
                .signing_result
                .as_ref()
                .map(|sr| sr.try_into())
                .transpose()?
                .ok_or(ServiceError::Generic("Signing result is none".to_string()))?;

            let signature = sign_aggregate_frost(SignAggregateFrostParams {
                signer: &self.signer,
                tx: &signing_job.tx,
                prev_out: &signing_job.parent_tx_out,
                signing_public_key: &signing_public_key,
                aggregating_public_key: &signing_public_key,
                signing_private_key: &signing_key,
                self_nonce_commitment: &signing_job.signing_commitments,
                adaptor_public_key: None,
                verifying_key: &verifying_key,
                signing_result,
            })
            .await?
            .serialize()?
            .to_vec();

            match signing_job.tx_type {
                SigningJobTxType::CpfpNode => {
                    node_signatures.node_tx_signature = signature;
                }
                SigningJobTxType::CpfpRefund => {
                    node_signatures.refund_tx_signature = signature;
                }
                SigningJobTxType::DirectNode => {
                    node_signatures.direct_node_tx_signature = signature;
                }
                SigningJobTxType::DirectRefund => {
                    node_signatures.direct_refund_tx_signature = signature;
                }
                SigningJobTxType::DirectFromCpfpRefund => {
                    node_signatures.direct_from_cpfp_refund_tx_signature = signature;
                }
            }
        }

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .finalize_node_signatures_v2(FinalizeNodeSignaturesRequest {
                intent: SignatureIntent::Refresh.into(),
                node_signatures: vec![node_signatures],
            })
            .await?;

        if response.nodes.len() != 1 {
            return Err(ServiceError::Generic(
                "Expected 1 node in response".to_string(),
            ));
        }

        response.nodes[0].clone().try_into()
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
            if node.needs_timelock_extension() {
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
                let our_extended_nodes = self
                    .transfer_service
                    .transfer_leaves_to_self(
                        extended_nodes,
                        Some(PrivateKeySource::Derived(node.id.clone())),
                    )
                    .await?;
                Ok::<Vec<TreeNode>, ServiceError>(our_extended_nodes)
            });
        }

        let extended_nodes = futures::future::try_join_all(extend_tasks).await?;
        ready_nodes.extend(extended_nodes.into_iter().flatten().collect::<Vec<_>>());

        Ok(ready_nodes)
    }

    /// Extends the timelock of a node by creating a new child node (this having the initial
    /// timelock on both node and refund txs).
    ///
    /// Unlike `refresh_timelock_node` which decreases timelocks on existing transactions,
    /// this function creates a completely new child node. Should be done when the node tx
    /// timelock is about to expire.
    ///
    /// Transaction Relationship Structure:
    /// ```ignore
    ///                          +------------------+
    ///                          | Original Node TX |
    ///                          +--------+---------+
    ///                                   |
    ///                     +-------------+--------------+
    ///                     |                            |
    ///           +---------v----------+       +---------v----------+
    ///           | New CPFP Node TX   |       | New Direct Node TX |
    ///           | (anchor, no fee)   |       | (no anchor, fee)   |
    ///           +---------+----------+       +---------+----------+
    ///                     |                            |
    ///      +--------------+-------------+              +----------+
    ///      |                            |                         |
    /// +----v-------------+      +-------v----------+       +------v-----------+
    /// | CPFP Refund TX   |      | Direct From CPFP |       | Direct Refund TX |
    /// | (anchor, no fee) |      | Refund TX        |       | (no anchor, fee) |
    /// |                  |      | (no anchor, fee) |       |                  |
    /// +------------------+      +------------------+       +------------------+
    /// ```
    ///
    /// The key differences from refresh_timelock_node:
    /// 1. Creates a completely new node rather than updating an existing node
    /// 2. Uses initial timelocks for all transactions (resetting the countdown)
    /// 3. Creates a child of the existing node instead of replacing it
    /// 4. Requires a transfer to self to update the signing key for the new node
    ///
    /// The function:
    /// 1. Creates new node transactions spending from the original node
    /// 2. Creates new refund transactions with initial (maximum) timelocks
    /// 3. Sets up signing commitments for all transactions
    /// 4. Signs all transactions using FROST threshold signatures
    /// 5. Finalizes the signatures with operators
    ///
    /// # Arguments
    ///
    /// * `node` - The node whose timelock needs to be extended
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<TreeNode>)` - The newly created tree nodes with fresh timelocks
    /// * `Err(ServiceError)` - If any part of the extension process fails
    pub async fn extend_time_lock(&self, node: &TreeNode) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Extending timelock node: {:?}", node.id);
        let signing_key = PrivateKeySource::Derived(node.id.clone());
        let signing_public_key = self
            .signer
            .get_public_key_from_private_key_source(&signing_key)?;

        let node_tx = &node.node_tx;
        let node_tx_out = &node_tx.output[0];
        let node_outpoint = OutPoint {
            txid: node_tx.compute_txid(),
            vout: 0,
        };

        let refund_tx = node
            .refund_tx
            .clone()
            .ok_or(ServiceError::Generic("No refund tx".to_string()))?;
        let refund_tx_out = &refund_tx.output[0];

        let (cpfp_sequence, direct_sequence) = next_sequence(refund_tx.input[0].sequence).ok_or(
            ServiceError::Generic("Failed to get next sequence".to_string()),
        )?;

        let NodeTransactions {
            cpfp_tx: cpfp_node_tx,
            direct_tx: direct_node_tx,
        } = create_node_txs(
            cpfp_sequence,
            direct_sequence,
            node_outpoint,
            Some(node_outpoint),
            node_tx_out.value,
            node_tx_out.script_pubkey.clone(),
            true,
        );

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: direct_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = create_refund_txs(
            initial_cpfp_sequence(),
            initial_direct_sequence(),
            OutPoint {
                txid: cpfp_node_tx.compute_txid(),
                vout: 0,
            },
            direct_node_tx.as_ref().map(|tx| OutPoint {
                txid: tx.compute_txid(),
                vout: 0,
            }),
            refund_tx_out.value.to_sat(),
            &signing_public_key,
            self.network,
        );

        let node_tx_signing_job = SigningJob {
            tx_type: SigningJobTxType::CpfpNode,
            tx: cpfp_node_tx.clone(),
            parent_tx_out: node_tx_out.clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_frost_signing_commitments().await?,
        };
        let refund_tx_signing_job = SigningJob {
            tx_type: SigningJobTxType::CpfpRefund,
            tx: cpfp_refund_tx,
            parent_tx_out: cpfp_node_tx.output[0].clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_frost_signing_commitments().await?,
        };

        let direct_node_tx_signing_job = if let Some(direct_node_tx) = &direct_node_tx {
            Some(SigningJob {
                tx_type: SigningJobTxType::DirectNode,
                tx: direct_node_tx.clone(),
                parent_tx_out: node_tx_out.clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_frost_signing_commitments().await?,
            })
        } else {
            None
        };

        let direct_refund_tx_signing_job = if let (Some(direct_refund_tx), Some(direct_node_tx)) =
            (direct_refund_tx, &direct_node_tx)
        {
            Some(SigningJob {
                tx_type: SigningJobTxType::DirectRefund,
                tx: direct_refund_tx.clone(),
                parent_tx_out: direct_node_tx.output[0].clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_frost_signing_commitments().await?,
            })
        } else {
            None
        };

        let direct_from_cpfp_refund_tx_signing_job =
            if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
                Some(SigningJob {
                    tx_type: SigningJobTxType::DirectFromCpfpRefund,
                    tx: direct_from_cpfp_refund_tx,
                    parent_tx_out: cpfp_node_tx.output[0].clone(),
                    signing_public_key,
                    signing_commitments: self.signer.generate_frost_signing_commitments().await?,
                })
            } else {
                None
            };

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .extend_leaf_v2(ExtendLeafRequest {
                leaf_id: node.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                node_tx_signing_job: Some(node_tx_signing_job.as_ref().try_into()?),
                refund_tx_signing_job: Some(refund_tx_signing_job.as_ref().try_into()?),
                direct_node_tx_signing_job: direct_node_tx_signing_job
                    .as_ref()
                    .map(|job| job.try_into())
                    .transpose()?,
                direct_refund_tx_signing_job: direct_refund_tx_signing_job
                    .as_ref()
                    .map(|job| job.try_into())
                    .transpose()?,
                direct_from_cpfp_refund_tx_signing_job: direct_from_cpfp_refund_tx_signing_job
                    .as_ref()
                    .map(|job| job.try_into())
                    .transpose()?,
            })
            .await?;

        let node_tx_extended_signing_result: ExtendLeafSigningResult = response
            .node_tx_signing_result
            .as_ref()
            .map(|sr| sr.try_into())
            .transpose()?
            .ok_or(ServiceError::Generic(
                "Node tx extended leaf signing result is none".to_string(),
            ))?;

        let refund_tx_extended_signing_result: ExtendLeafSigningResult = response
            .refund_tx_signing_result
            .as_ref()
            .map(|sr| sr.try_into())
            .transpose()?
            .ok_or(ServiceError::Generic(
                "Refund tx extended leaf signing result is none".to_string(),
            ))?;

        let node_tx_signature = sign_aggregate_frost(SignAggregateFrostParams {
            signer: &self.signer,
            tx: &node_tx_signing_job.tx,
            prev_out: &node_tx_signing_job.parent_tx_out,
            signing_public_key: &signing_public_key,
            aggregating_public_key: &signing_public_key,
            signing_private_key: &signing_key,
            self_nonce_commitment: &node_tx_signing_job.signing_commitments,
            adaptor_public_key: None,
            verifying_key: &node_tx_extended_signing_result.verifying_key,
            signing_result: node_tx_extended_signing_result.signing_result.ok_or(
                ServiceError::Generic("Node tx signing result is none".to_string()),
            )?,
        })
        .await?;

        let refund_tx_signature = sign_aggregate_frost(SignAggregateFrostParams {
            signer: &self.signer,
            tx: &refund_tx_signing_job.tx,
            prev_out: &refund_tx_signing_job.parent_tx_out,
            signing_public_key: &signing_public_key,
            aggregating_public_key: &signing_public_key,
            signing_private_key: &signing_key,
            self_nonce_commitment: &refund_tx_signing_job.signing_commitments,
            adaptor_public_key: None,
            verifying_key: &refund_tx_extended_signing_result.verifying_key,
            signing_result: refund_tx_extended_signing_result.signing_result.ok_or(
                ServiceError::Generic("Refund tx signing result is none".to_string()),
            )?,
        })
        .await?;

        let mut node_signatures = NodeSignatures {
            node_id: response.leaf_id,
            node_tx_signature: node_tx_signature.serialize()?.to_vec(),
            refund_tx_signature: refund_tx_signature.serialize()?.to_vec(),
            ..Default::default()
        };

        if let Some(direct_node_tx_signing_job) = direct_node_tx_signing_job {
            let direct_node_tx_extended_signing_result: ExtendLeafSigningResult = response
                .direct_node_tx_signing_result
                .as_ref()
                .map(|sr| sr.try_into())
                .transpose()?
                .ok_or(ServiceError::Generic(
                    "Direct node tx extended leaf signing result is none".to_string(),
                ))?;

            let direct_node_tx_signature = sign_aggregate_frost(SignAggregateFrostParams {
                signer: &self.signer,
                tx: &direct_node_tx_signing_job.tx,
                prev_out: &direct_node_tx_signing_job.parent_tx_out,
                signing_public_key: &signing_public_key,
                aggregating_public_key: &signing_public_key,
                signing_private_key: &signing_key,
                self_nonce_commitment: &direct_node_tx_signing_job.signing_commitments,
                adaptor_public_key: None,
                verifying_key: &direct_node_tx_extended_signing_result.verifying_key,
                signing_result: direct_node_tx_extended_signing_result
                    .signing_result
                    .ok_or(ServiceError::Generic(
                        "Direct node tx signing result is none".to_string(),
                    ))?,
            })
            .await?;

            node_signatures.direct_node_tx_signature =
                direct_node_tx_signature.serialize()?.to_vec();
        }

        if let Some(direct_refund_tx_signing_job) = direct_refund_tx_signing_job {
            let direct_refund_tx_extended_signing_result: ExtendLeafSigningResult = response
                .direct_refund_tx_signing_result
                .as_ref()
                .map(|sr| sr.try_into())
                .transpose()?
                .ok_or(ServiceError::Generic(
                    "Direct refund tx extended leaf signing result is none".to_string(),
                ))?;

            let direct_refund_tx_signature = sign_aggregate_frost(SignAggregateFrostParams {
                signer: &self.signer,
                tx: &direct_refund_tx_signing_job.tx,
                prev_out: &direct_refund_tx_signing_job.parent_tx_out,
                signing_public_key: &signing_public_key,
                aggregating_public_key: &signing_public_key,
                signing_private_key: &signing_key,
                self_nonce_commitment: &direct_refund_tx_signing_job.signing_commitments,
                adaptor_public_key: None,
                verifying_key: &direct_refund_tx_extended_signing_result.verifying_key,
                signing_result: direct_refund_tx_extended_signing_result
                    .signing_result
                    .ok_or(ServiceError::Generic(
                        "Direct refund tx signing result is none".to_string(),
                    ))?,
            })
            .await?;

            node_signatures.direct_refund_tx_signature =
                direct_refund_tx_signature.serialize()?.to_vec();
        }

        if let Some(direct_from_cpfp_refund_tx_signing_job) = direct_from_cpfp_refund_tx_signing_job
        {
            let direct_from_cpfp_refund_tx_extended_signing_result: ExtendLeafSigningResult =
                response
                    .direct_from_cpfp_refund_tx_signing_result
                    .as_ref()
                    .map(|sr| sr.try_into())
                    .transpose()?
                    .ok_or(ServiceError::Generic(
                        "Direct from cpfp refund tx extended leaf signing result is none"
                            .to_string(),
                    ))?;

            let direct_from_cpfp_refund_tx_signature =
                sign_aggregate_frost(SignAggregateFrostParams {
                    signer: &self.signer,
                    tx: &direct_from_cpfp_refund_tx_signing_job.tx,
                    prev_out: &direct_from_cpfp_refund_tx_signing_job.parent_tx_out,
                    signing_public_key: &signing_public_key,
                    aggregating_public_key: &signing_public_key,
                    signing_private_key: &signing_key,
                    self_nonce_commitment: &direct_from_cpfp_refund_tx_signing_job
                        .signing_commitments,
                    adaptor_public_key: None,
                    verifying_key: &direct_from_cpfp_refund_tx_extended_signing_result
                        .verifying_key,
                    signing_result: direct_from_cpfp_refund_tx_extended_signing_result
                        .signing_result
                        .ok_or(ServiceError::Generic(
                            "Direct from cpfp refund tx signing result is none".to_string(),
                        ))?,
                })
                .await?;

            node_signatures.direct_from_cpfp_refund_tx_signature =
                direct_from_cpfp_refund_tx_signature.serialize()?.to_vec();
        }

        let nodes = self
            .operator_pool
            .get_coordinator()
            .client
            .finalize_node_signatures_v2(FinalizeNodeSignaturesRequest {
                intent: SignatureIntent::Extend.into(),
                node_signatures: vec![node_signatures],
            })
            .await?
            .nodes;

        nodes
            .into_iter()
            .map(|n| n.try_into())
            .collect::<Result<Vec<TreeNode>, _>>()
    }
}
