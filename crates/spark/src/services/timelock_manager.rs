use std::{collections::HashMap, sync::Arc};

use tracing::{error, info, trace};

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            QueryNodesPaginatedRequest,
            spark::{
                GetSigningCommitmentsRequest, RenewLeafRequest, RenewNodeTimelockSigningJob,
                RenewNodeZeroTimelockSigningJob, RenewRefundTimelockSigningJob, TreeNodeIds,
                query_nodes_request::Source, renew_leaf_request::SigningJobs,
                renew_leaf_response::RenewResult,
            },
        },
    },
    services::{ServiceError, map_signing_nonce_commitments},
    signer::SparkSigner,
    tree::{LeafPedigree, TreeNode, TreeNodeId, assemble_exit_chains},
    utils::{
        signing_job::{SigningJob, SigningJobType, sign_signing_jobs},
        transactions::{
            NodeTransactions, RefundTransactions, create_decremented_timelock_node_txs,
            create_initial_timelock_node_txs, create_initial_timelock_refund_txs,
            create_zero_timelock_node_txs,
        },
    },
};
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments};
use std::collections::BTreeMap;
pub struct TimelockManager {
    spark_signer: Arc<dyn SparkSigner>,
    network: Network,
    operator_pool: Arc<OperatorPool>,
}

impl TimelockManager {
    pub fn new(
        spark_signer: Arc<dyn SparkSigner>,
        network: Network,
        operator_pool: Arc<OperatorPool>,
    ) -> Self {
        Self {
            spark_signer,
            network,
            operator_pool,
        }
    }

    async fn get_signing_commitments_for_jobs(
        &self,
        node_id: &TreeNodeId,
        signing_jobs_count: usize,
    ) -> Result<Vec<BTreeMap<Identifier, SigningCommitments>>, ServiceError> {
        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(GetSigningCommitmentsRequest {
                node_ids: vec![node_id.to_string()],
                count: signing_jobs_count as u32,
                node_id_count: 0,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(signing_commitments)
    }

    /// Renews any leaf whose refund timelock is expiring, returning each leaf with
    /// its ancestor chain. An unrenewed leaf passes through unchanged. A renewed leaf
    /// carries its new chain, rebuilt in memory from the ancestors it came in with
    /// plus the new split node the coordinator returns. The parent that drives the
    /// renewal normally comes from the pedigree; if the stored chain is incomplete it
    /// is fetched from the operators as a fallback.
    pub async fn check_renew_nodes(
        &self,
        pedigrees: Vec<LeafPedigree>,
    ) -> Result<Vec<LeafPedigree>, ServiceError> {
        trace!("Checking renew nodes: {:?}", pedigrees);
        let mut ready = Vec::new();
        let mut renewable = Vec::new();
        for pedigree in pedigrees {
            if pedigree.leaf.needs_refund_tx_renewed()? {
                renewable.push(pedigree);
            } else {
                ready.push(pedigree);
            }
        }

        if renewable.is_empty() {
            return Ok(ready);
        }

        let fetched = self.fetch_missing_renewal_parents(&renewable).await?;
        let renew_futures = renewable
            .iter()
            .map(|pedigree| self.renew_pedigree(pedigree, &fetched));
        // One leaf the operators will not renew does not fail the batch: it is
        // kept as it arrived and tried again on the next pass.
        let renew_results = futures::future::join_all(renew_futures).await;
        for (pedigree, result) in renewable.into_iter().zip(renew_results) {
            match result {
                Ok(renewed) => ready.push(renewed),
                Err(e) => {
                    error!(
                        "Timelock renewal failed for leaf {}, keeping it unrenewed for the next pass: {e:?}",
                        pedigree.leaf.id
                    );
                    ready.push(pedigree);
                }
            }
        }
        Ok(ready)
    }

    /// Fetches the ancestors of every leaf about to be renewed whose stored chain does
    /// not already carry its parent, in a single query. Batched rather than resolved
    /// per leaf: the leaves in a wallet tend to age together, so a query each would
    /// multiply an operation's round trips by the number of timelocks coming due at
    /// once. Returns an empty map when every parent is already stored, which is the
    /// usual case and costs no call at all.
    async fn fetch_missing_renewal_parents(
        &self,
        renewable: &[LeafPedigree],
    ) -> Result<HashMap<TreeNodeId, TreeNode>, ServiceError> {
        let node_ids: Vec<String> = renewable
            .iter()
            // A zero-timelock renewal builds on no parent.
            .filter(|pedigree| !pedigree.leaf.is_zero_timelock())
            .filter(|pedigree| match &pedigree.leaf.parent_node_id {
                Some(parent_id) => !pedigree.ancestors.iter().any(|a| &a.id == parent_id),
                None => false,
            })
            .map(|pedigree| pedigree.leaf.id.to_string())
            .collect();
        if node_ids.is_empty() {
            return Ok(HashMap::new());
        }
        trace!(
            "Fetching renewal parents for {} leaves whose stored chain lacks one",
            node_ids.len()
        );

        let result = self
            .operator_pool
            .get_coordinator()
            .client
            .query_nodes_paginated(
                QueryNodesPaginatedRequest {
                    source: Some(Source::NodeIds(TreeNodeIds { node_ids })),
                    include_parents: true,
                    network: self.network.to_proto_network().into(),
                    ..Default::default()
                },
                None,
            )
            .await?;
        let mut nodes = HashMap::new();
        for (_id, node) in result.items {
            let node: TreeNode = node.try_into()?;
            nodes.insert(node.id.clone(), node);
        }
        Ok(nodes)
    }

    /// Renews one leaf and rebuilds its pedigree in memory. The renewal may reparent
    /// the leaf onto a new split node (returned by the coordinator); the rest of the
    /// chain is unchanged and comes from the pedigree the leaf arrived with.
    async fn renew_pedigree(
        &self,
        pedigree: &LeafPedigree,
        fetched: &HashMap<TreeNodeId, TreeNode>,
    ) -> Result<LeafPedigree, ServiceError> {
        let leaf = &pedigree.leaf;
        let mut nodes: HashMap<TreeNodeId, TreeNode> = pedigree
            .ancestors
            .iter()
            .map(|a| (a.id.clone(), a.clone()))
            .collect();

        let (renewed_leaf, split_node) = if leaf.is_zero_timelock() {
            self.renew_zero_timelock(leaf).await?
        } else {
            let parent = Self::resolve_renewal_parent(leaf, &mut nodes, fetched)?;
            if leaf.needs_node_tx_renewed() {
                self.renew_node(leaf, &parent).await?
            } else {
                self.renew_refund(leaf, &parent).await?
            }
        };

        if let Some(split_node) = split_node {
            nodes.insert(split_node.id.clone(), split_node);
        }
        nodes.insert(renewed_leaf.id.clone(), renewed_leaf.clone());
        assemble_exit_chains(&nodes, std::slice::from_ref(&renewed_leaf.id))
            .pop()
            .ok_or_else(|| {
                ServiceError::Generic(format!(
                    "Failed to rebuild chain for node {}",
                    renewed_leaf.id
                ))
            })
    }

    /// Resolves the parent whose `node_tx` a non-zero-timelock renewal builds on.
    /// It is normally already in the pedigree; when the stored chain is incomplete
    /// (e.g. a leaf claimed while the coordinator was unreachable) it comes from the
    /// batch fetched up front, whose chain is walked in so the pedigree rebuilt after
    /// the renewal is complete too. Errors only if the parent is in neither.
    fn resolve_renewal_parent(
        leaf: &TreeNode,
        nodes: &mut HashMap<TreeNodeId, TreeNode>,
        fetched: &HashMap<TreeNodeId, TreeNode>,
    ) -> Result<TreeNode, ServiceError> {
        let parent_id = leaf
            .parent_node_id
            .clone()
            .ok_or_else(|| ServiceError::Generic(format!("Node {} has no parent node", leaf.id)))?;
        if !nodes.contains_key(&parent_id) {
            // Only this leaf's own line, not every node the batch returned.
            let mut current = Some(parent_id.clone());
            while let Some(id) = current {
                if nodes.contains_key(&id) {
                    break;
                }
                let Some(ancestor) = fetched.get(&id) else {
                    break;
                };
                current = ancestor.parent_node_id.clone();
                nodes.insert(id, ancestor.clone());
            }
        }
        nodes.get(&parent_id).cloned().ok_or_else(|| {
            ServiceError::Generic(format!("Parent node not found for node {}", leaf.id))
        })
    }

    async fn renew_node(
        &self,
        node: &TreeNode,
        parent_node: &TreeNode,
    ) -> Result<(TreeNode, Option<TreeNode>), ServiceError> {
        info!("Renewing node: {:?}", node.id);
        let mut signing_jobs = Vec::new();

        // Fetch the signing key from the signer, never derived from persisted
        // tree data: the renewed refund pays to this key, so a coordinator that
        // lied about the stored keyshare pubkey could otherwise steer the exit
        // refund to a key it controls.
        let signing_public_key = self.spark_signer.get_public_key_for_leaf(&node.id).await?;

        let parent_node_tx = &parent_node.node_tx;

        let NodeTransactions {
            cpfp_tx: cpfp_split_node_tx,
            direct_tx: direct_split_node_tx,
        } = create_zero_timelock_node_txs(parent_node_tx)?;

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpSplitNode,
            node_id: node.id.clone(),
            tx: cpfp_split_node_tx.clone(),
            parent_tx_out: parent_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectSplitNode,
            node_id: node.id.clone(),
            tx: direct_split_node_tx.clone(),
            parent_tx_out: parent_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        let NodeTransactions {
            cpfp_tx: cpfp_node_tx,
            direct_tx: direct_node_tx,
        } = create_initial_timelock_node_txs(&cpfp_split_node_tx)?;

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpNode,
            node_id: node.id.clone(),
            tx: cpfp_node_tx.clone(),
            parent_tx_out: cpfp_split_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectNode,
            node_id: node.id.clone(),
            tx: direct_node_tx.clone(),
            parent_tx_out: cpfp_split_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: direct_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = create_initial_timelock_refund_txs(
            &cpfp_node_tx,
            Some(&direct_node_tx),
            &signing_public_key,
            self.network,
        );

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpRefund,
            node_id: node.id.clone(),
            tx: cpfp_refund_tx,
            parent_tx_out: cpfp_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        if let Some(direct_refund_tx) = direct_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectRefund,
                node_id: node.id.clone(),
                tx: direct_refund_tx.clone(),
                parent_tx_out: direct_node_tx.output[0].clone(),
                signing_public_key,
                verifying_public_key: node.verifying_public_key,
            });
        }

        if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectFromCpfpRefund,
                node_id: node.id.clone(),
                tx: direct_from_cpfp_refund_tx,
                parent_tx_out: cpfp_node_tx.output[0].clone(),
                signing_public_key,
                verifying_public_key: node.verifying_public_key,
            });
        }

        let signing_commitments = self
            .get_signing_commitments_for_jobs(&node.id, signing_jobs.len())
            .await?;

        let signed_jobs = sign_signing_jobs(
            &self.spark_signer,
            signing_jobs,
            signing_commitments,
            self.network,
        )
        .await?;

        let idempotency_key = node
            .refund_tx
            .as_ref()
            .map(|tx| tx.compute_txid().to_string());

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .renew_leaf(
                RenewLeafRequest {
                    leaf_id: node.id.to_string(),
                    signing_jobs: Some(SigningJobs::RenewNodeTimelockSigningJob(
                        RenewNodeTimelockSigningJob {
                            split_node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpSplitNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            split_node_direct_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectSplitNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_from_cpfp_refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectFromCpfpRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                        },
                    )),
                },
                idempotency_key,
            )
            .await?;

        let Some(RenewResult::RenewNodeTimelockResult(renew_result)) = response.renew_result else {
            return Err(ServiceError::Generic(
                "Expected renew node timelock reponse".to_string(),
            ));
        };

        // The renewal re-splits from the parent, so the response carries the new
        // split node the leaf is now parented onto.
        let node = renew_result
            .node
            .ok_or(ServiceError::Generic(
                "Expected a node in response".to_string(),
            ))?
            .try_into()?;
        let split_node = renew_result.split_node.map(TryInto::try_into).transpose()?;
        Ok((node, split_node))
    }

    async fn renew_refund(
        &self,
        node: &TreeNode,
        parent_node: &TreeNode,
    ) -> Result<(TreeNode, Option<TreeNode>), ServiceError> {
        info!("Renewing refund: {:?}", node.id);
        let mut signing_jobs = Vec::new();

        // Fetch the signing key from the signer, never derived from persisted
        // tree data: the renewed refund pays to this key, so a coordinator that
        // lied about the stored keyshare pubkey could otherwise steer the exit
        // refund to a key it controls.
        let signing_public_key = self.spark_signer.get_public_key_for_leaf(&node.id).await?;

        let parent_node_tx = &parent_node.node_tx;
        let node_tx = &node.node_tx;

        let NodeTransactions {
            cpfp_tx: cpfp_node_tx,
            direct_tx: direct_node_tx,
        } = create_decremented_timelock_node_txs(parent_node_tx, node_tx)?;

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpNode,
            node_id: node.id.clone(),
            tx: cpfp_node_tx.clone(),
            parent_tx_out: parent_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectNode,
            node_id: node.id.clone(),
            tx: direct_node_tx.clone(),
            parent_tx_out: parent_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_tx: direct_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
        } = create_initial_timelock_refund_txs(
            &cpfp_node_tx,
            Some(&direct_node_tx),
            &signing_public_key,
            self.network,
        );

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpRefund,
            node_id: node.id.clone(),
            tx: cpfp_refund_tx,
            parent_tx_out: cpfp_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        if let Some(direct_refund_tx) = direct_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectRefund,
                node_id: node.id.clone(),
                tx: direct_refund_tx.clone(),
                parent_tx_out: direct_node_tx.output[0].clone(),
                signing_public_key,
                verifying_public_key: node.verifying_public_key,
            });
        }

        if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectFromCpfpRefund,
                node_id: node.id.clone(),
                tx: direct_from_cpfp_refund_tx,
                parent_tx_out: cpfp_node_tx.output[0].clone(),
                signing_public_key,
                verifying_public_key: node.verifying_public_key,
            });
        }

        let signing_commitments = self
            .get_signing_commitments_for_jobs(&node.id, signing_jobs.len())
            .await?;

        let signed_jobs = sign_signing_jobs(
            &self.spark_signer,
            signing_jobs,
            signing_commitments,
            self.network,
        )
        .await?;

        let idempotency_key = node
            .refund_tx
            .as_ref()
            .map(|tx| tx.compute_txid().to_string());

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .renew_leaf(
                RenewLeafRequest {
                    leaf_id: node.id.to_string(),
                    signing_jobs: Some(SigningJobs::RenewRefundTimelockSigningJob(
                        RenewRefundTimelockSigningJob {
                            node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_from_cpfp_refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectFromCpfpRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                        },
                    )),
                },
                idempotency_key,
            )
            .await?;

        let Some(RenewResult::RenewRefundTimelockResult(renew_result)) = response.renew_result
        else {
            return Err(ServiceError::Generic(
                "Expected renew node timelock reponse".to_string(),
            ));
        };

        // A refund-only renewal keeps the leaf under the same parent, so there is no
        // new split node.
        let node = renew_result
            .node
            .ok_or(ServiceError::Generic(
                "Expected a node in response".to_string(),
            ))?
            .try_into()?;
        Ok((node, None))
    }

    pub async fn renew_zero_timelock(
        &self,
        node: &TreeNode,
    ) -> Result<(TreeNode, Option<TreeNode>), ServiceError> {
        info!("Renewing zero timelock: {:?}", node.id);
        let mut signing_jobs = Vec::new();

        // Fetch the signing key from the signer, never derived from persisted
        // tree data: the renewed refund pays to this key, so a coordinator that
        // lied about the stored keyshare pubkey could otherwise steer the exit
        // refund to a key it controls.
        let signing_public_key = self.spark_signer.get_public_key_for_leaf(&node.id).await?;

        let node_tx = &node.node_tx;

        let NodeTransactions {
            cpfp_tx: cpfp_node_tx,
            direct_tx: direct_node_tx,
        } = create_zero_timelock_node_txs(node_tx)?;

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpNode,
            node_id: node.id.clone(),
            tx: cpfp_node_tx.clone(),
            parent_tx_out: node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectNode,
            node_id: node.id.clone(),
            tx: direct_node_tx.clone(),
            parent_tx_out: node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        let RefundTransactions {
            cpfp_tx: cpfp_refund_tx,
            direct_from_cpfp_tx: direct_from_cpfp_refund_tx,
            ..
        } = create_initial_timelock_refund_txs(
            &cpfp_node_tx,
            Some(&direct_node_tx),
            &signing_public_key,
            self.network,
        );

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::CpfpRefund,
            node_id: node.id.clone(),
            tx: cpfp_refund_tx,
            parent_tx_out: cpfp_node_tx.output[0].clone(),
            signing_public_key,
            verifying_public_key: node.verifying_public_key,
        });

        if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectFromCpfpRefund,
                node_id: node.id.clone(),
                tx: direct_from_cpfp_refund_tx,
                parent_tx_out: cpfp_node_tx.output[0].clone(),
                signing_public_key,
                verifying_public_key: node.verifying_public_key,
            });
        }

        let signing_commitments = self
            .get_signing_commitments_for_jobs(&node.id, signing_jobs.len())
            .await?;

        let signed_jobs = sign_signing_jobs(
            &self.spark_signer,
            signing_jobs,
            signing_commitments,
            self.network,
        )
        .await?;

        let idempotency_key = node
            .refund_tx
            .as_ref()
            .map(|tx| tx.compute_txid().to_string());

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .renew_leaf(
                RenewLeafRequest {
                    leaf_id: node.id.to_string(),
                    signing_jobs: Some(SigningJobs::RenewNodeZeroTimelockSigningJob(
                        RenewNodeZeroTimelockSigningJob {
                            node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::CpfpRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_node_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectNode)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                            direct_from_cpfp_refund_tx_signing_job: signed_jobs
                                .iter()
                                .find(|j| j.job_type == SigningJobType::DirectFromCpfpRefund)
                                .map(|j| j.signed_tx.as_ref().try_into())
                                .transpose()?,
                        },
                    )),
                },
                idempotency_key,
            )
            .await?;

        let Some(RenewResult::RenewNodeZeroTimelockResult(renew_result)) = response.renew_result
        else {
            return Err(ServiceError::Generic(
                "Expected renew node timelock reponse".to_string(),
            ));
        };

        let node = renew_result
            .node
            .ok_or(ServiceError::Generic(
                "Expected a node in response".to_string(),
            ))?
            .try_into()?;
        let split_node = renew_result.split_node.map(TryInto::try_into).transpose()?;
        Ok((node, split_node))
    }
}
