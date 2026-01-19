use std::{collections::HashMap, str::FromStr as _, sync::Arc};

use tracing::{info, trace};

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::spark::{
            GetSigningCommitmentsRequest, RenewLeafRequest, RenewNodeTimelockSigningJob,
            RenewNodeZeroTimelockSigningJob, RenewRefundTimelockSigningJob, TreeNodeIds,
            query_nodes_request::Source, renew_leaf_request::SigningJobs,
            renew_leaf_response::RenewResult,
        },
    },
    services::{ServiceError, map_signing_nonce_commitments},
    signer::{SecretSource, Signer},
    tree::{TreeNode, TreeNodeId},
    utils::{
        signing_job::{SigningJob, SigningJobType, sign_signing_jobs},
        transactions::{
            NodeTransactions, RefundTransactions, create_decremented_timelock_node_txs,
            create_initial_timelock_node_txs, create_initial_timelock_refund_txs,
            create_zero_timelock_node_txs,
        },
    },
};

enum RenewType<'a> {
    Node { parent_node: &'a TreeNode },
    Refund { parent_node: &'a TreeNode },
    ZeroTimelock,
}

pub struct TimelockManager {
    signer: Arc<dyn Signer>,
    network: Network,
    operator_pool: Arc<OperatorPool>,
}

impl TimelockManager {
    pub fn new(
        signer: Arc<dyn Signer>,
        network: Network,
        operator_pool: Arc<OperatorPool>,
    ) -> Self {
        Self {
            signer,
            network,
            operator_pool,
        }
    }

    pub async fn check_renew_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Checking renew nodes: {:?}", nodes);
        let mut renewable_nodes = Vec::new();
        let mut renewable_refunds = Vec::new();
        let mut renewable_zero_timelock_nodes = Vec::new();
        let mut node_ids = Vec::new();
        let mut ready_nodes = Vec::new();

        for node in nodes {
            if node.needs_refund_tx_renewed()? {
                node_ids.push(node.id.to_string());
                if node.is_zero_timelock() {
                    renewable_zero_timelock_nodes.push(node);
                } else if node.needs_node_tx_renewed() {
                    renewable_nodes.push(node);
                } else {
                    renewable_refunds.push(node);
                }
            } else {
                ready_nodes.push(node);
            }
        }

        if renewable_nodes.is_empty()
            && renewable_refunds.is_empty()
            && renewable_zero_timelock_nodes.is_empty()
        {
            return Ok(ready_nodes);
        }

        // Get the parent nodes
        let paging_result = self
            .operator_pool
            .get_coordinator()
            .client
            .query_nodes_paginated(
                crate::operator::rpc::QueryNodesPaginatedRequest {
                    source: Some(Source::NodeIds(TreeNodeIds { node_ids })),
                    include_parents: true,
                    network: self.network.to_proto_network() as i32,
                    ..Default::default()
                },
                None, // fetch all pages
            )
            .await?;

        let mut node_ids_to_nodes_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();
        for (_node_id_str, node) in paging_result.items {
            node_ids_to_nodes_map.insert(
                TreeNodeId::from_str(&node.id).map_err(ServiceError::ValidationError)?,
                node.clone().try_into()?,
            );
        }

        let get_parent_node = |node: &TreeNode| -> Result<&TreeNode, ServiceError> {
            node_ids_to_nodes_map
                .get(
                    &node
                        .parent_node_id
                        .clone()
                        .ok_or(ServiceError::Generic(format!(
                            "Node {} has no parent node",
                            node.id
                        )))?,
                )
                .ok_or(ServiceError::Generic(format!(
                    "Parent node not found for node {}",
                    node.id
                )))
        };

        let mut renew_futures = Vec::new();
        for node in &renewable_nodes {
            let parent_node = get_parent_node(node)?;
            renew_futures.push(self.renew(RenewType::Node { parent_node }, node));
        }
        for node in &renewable_refunds {
            let parent_node = get_parent_node(node)?;
            renew_futures.push(self.renew(RenewType::Refund { parent_node }, node));
        }
        for node in &renewable_zero_timelock_nodes {
            renew_futures.push(self.renew(RenewType::ZeroTimelock, node));
        }

        let renewed_nodes = futures::future::try_join_all(renew_futures).await?;
        ready_nodes.extend(renewed_nodes);

        Ok(ready_nodes)
    }

    async fn renew<'a>(
        &self,
        renew_type: RenewType<'a>,
        node: &TreeNode,
    ) -> Result<TreeNode, ServiceError> {
        match renew_type {
            RenewType::Node { parent_node } => self.renew_node(node, parent_node).await,
            RenewType::Refund { parent_node } => self.renew_refund(node, parent_node).await,
            RenewType::ZeroTimelock => self.renew_zero_timelock(node).await,
        }
    }

    async fn renew_node(
        &self,
        node: &TreeNode,
        parent_node: &TreeNode,
    ) -> Result<TreeNode, ServiceError> {
        info!("Renewing node: {:?}", node.id);
        let mut signing_jobs = Vec::new();

        let signing_key = SecretSource::Derived(node.id.clone());
        let signing_public_key = self.signer.public_key_from_secret(&signing_key).await?;

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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectSplitNode,
            node_id: node.id.clone(),
            tx: direct_split_node_tx.clone(),
            parent_tx_out: parent_node_tx.output[0].clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectNode,
            node_id: node.id.clone(),
            tx: direct_node_tx.clone(),
            parent_tx_out: cpfp_split_node_tx.output[0].clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        if let Some(direct_refund_tx) = direct_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectRefund,
                node_id: node.id.clone(),
                tx: direct_refund_tx.clone(),
                parent_tx_out: direct_node_tx.output[0].clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_random_signing_commitment().await?,
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
                signing_commitments: self.signer.generate_random_signing_commitment().await?,
                verifying_public_key: node.verifying_public_key,
            });
        }

        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(GetSigningCommitmentsRequest {
                node_ids: vec![node.id.to_string()],
                count: signing_jobs.len() as u32,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let signed_jobs = sign_signing_jobs(
            &self.signer,
            signing_jobs,
            signing_commitments,
            self.network,
        )
        .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .renew_leaf(RenewLeafRequest {
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
            })
            .await?;

        let Some(RenewResult::RenewNodeTimelockResult(renew_result)) = response.renew_result else {
            return Err(ServiceError::Generic(
                "Expected renew node timelock reponse".to_string(),
            ));
        };

        renew_result
            .node
            .ok_or(ServiceError::Generic(
                "Expected a node in response".to_string(),
            ))?
            .try_into()
    }

    async fn renew_refund(
        &self,
        node: &TreeNode,
        parent_node: &TreeNode,
    ) -> Result<TreeNode, ServiceError> {
        info!("Renewing refund: {:?}", node.id);
        let mut signing_jobs = Vec::new();

        let signing_key = SecretSource::Derived(node.id.clone());
        let signing_public_key = self.signer.public_key_from_secret(&signing_key).await?;

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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectNode,
            node_id: node.id.clone(),
            tx: direct_node_tx.clone(),
            parent_tx_out: parent_node_tx.output[0].clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        if let Some(direct_refund_tx) = direct_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectRefund,
                node_id: node.id.clone(),
                tx: direct_refund_tx.clone(),
                parent_tx_out: direct_node_tx.output[0].clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_random_signing_commitment().await?,
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
                signing_commitments: self.signer.generate_random_signing_commitment().await?,
                verifying_public_key: node.verifying_public_key,
            });
        }

        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(GetSigningCommitmentsRequest {
                node_ids: vec![node.id.to_string()],
                count: signing_jobs.len() as u32,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let signed_jobs = sign_signing_jobs(
            &self.signer,
            signing_jobs,
            signing_commitments,
            self.network,
        )
        .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .renew_leaf(RenewLeafRequest {
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
            })
            .await?;

        let Some(RenewResult::RenewRefundTimelockResult(renew_result)) = response.renew_result
        else {
            return Err(ServiceError::Generic(
                "Expected renew node timelock reponse".to_string(),
            ));
        };

        renew_result
            .node
            .ok_or(ServiceError::Generic(
                "Expected a node in response".to_string(),
            ))?
            .try_into()
    }

    pub async fn renew_zero_timelock(&self, node: &TreeNode) -> Result<TreeNode, ServiceError> {
        info!("Renewing zero timelock: {:?}", node.id);
        let mut signing_jobs = Vec::new();

        let signing_key = SecretSource::Derived(node.id.clone());
        let signing_public_key = self.signer.public_key_from_secret(&signing_key).await?;

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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        signing_jobs.push(SigningJob {
            job_type: SigningJobType::DirectNode,
            node_id: node.id.clone(),
            tx: direct_node_tx.clone(),
            parent_tx_out: node_tx.output[0].clone(),
            signing_public_key,
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
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
            signing_commitments: self.signer.generate_random_signing_commitment().await?,
            verifying_public_key: node.verifying_public_key,
        });

        if let Some(direct_from_cpfp_refund_tx) = direct_from_cpfp_refund_tx {
            signing_jobs.push(SigningJob {
                job_type: SigningJobType::DirectFromCpfpRefund,
                node_id: node.id.clone(),
                tx: direct_from_cpfp_refund_tx,
                parent_tx_out: cpfp_node_tx.output[0].clone(),
                signing_public_key,
                signing_commitments: self.signer.generate_random_signing_commitment().await?,
                verifying_public_key: node.verifying_public_key,
            });
        }

        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(GetSigningCommitmentsRequest {
                node_ids: vec![node.id.to_string()],
                count: signing_jobs.len() as u32,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let signed_jobs = sign_signing_jobs(
            &self.signer,
            signing_jobs,
            signing_commitments,
            self.network,
        )
        .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .renew_leaf(RenewLeafRequest {
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
            })
            .await?;

        let Some(RenewResult::RenewNodeZeroTimelockResult(renew_result)) = response.renew_result
        else {
            return Err(ServiceError::Generic(
                "Expected renew node timelock reponse".to_string(),
            ));
        };

        renew_result
            .node
            .ok_or(ServiceError::Generic(
                "Expected a node in response".to_string(),
            ))?
            .try_into()
    }
}
