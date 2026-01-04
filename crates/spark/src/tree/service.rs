use futures::future::join_all;
use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;
use tracing::{error, info, trace, warn};

use crate::tree::{Leaves, ReservationPurpose, TreeNodeStatus};
use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            SparkRpcClient,
            spark::{QueryNodesRequest, query_nodes_request::Source},
        },
    },
    services::{ServiceError, Swap, TimelockManager},
    signer::Signer,
    tree::{
        LeavesReservation, LeavesReservationId, TargetAmounts, TreeNodeId, TreeService, TreeStore,
        select_helper, with_reserved_leaves,
    },
    utils::paging::{PagingFilter, PagingResult, pager},
};

use super::{TreeNode, error::TreeServiceError};

pub struct SynchronousTreeService {
    identity_pubkey: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool>,
    state: Arc<dyn TreeStore>,
    timelock_manager: Arc<TimelockManager>,
    signer: Arc<dyn Signer>,
    swap_service: Arc<Swap>,
}

#[macros::async_trait]
impl TreeService for SynchronousTreeService {
    async fn list_leaves(&self) -> Result<Leaves, TreeServiceError> {
        self.state.get_leaves().await
    }

    async fn cancel_reservation(&self, id: LeavesReservationId) -> Result<(), TreeServiceError> {
        self.state.cancel_reservation(&id).await
    }

    async fn finalize_reservation(
        &self,
        id: LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        self.state.finalize_reservation(&id, new_leaves).await
    }

    async fn insert_leaves(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let result_nodes = self
            .check_renew_nodes(leaves, async |e| {
                // If this is a partial check timelock error, the extend node timelock failed
                // but we can still update the leaves that were refreshed
                if let ServiceError::PartialCheckTimelockError(ref nodes) = e
                    && let Err(e) = self.state.add_leaves(nodes).await
                {
                    error!("Failed to add leaves: {e:?}");
                }
            })
            .await?;

        self.state.add_leaves(&result_nodes).await?;
        Ok(result_nodes)
    }

    /// Selects leaves from the tree that sum up to exactly the target amounts.
    /// If such a combination of leaves does not exist, it performs a swap to get a set of leaves matching the target amounts.
    /// If no leaves can be selected, returns an error
    async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!("Selecting leaves for target amounts: {target_amounts:?}, purpose: {purpose:?}");

        let reservation = self
            .reserve_fresh_leaves(target_amounts, false, purpose)
            .await?;

        let Some(reservation) = reservation else {
            return Err(TreeServiceError::InsufficientFunds);
        };

        trace!(
            "Selected leaves got reservation: {:?} ({})",
            reservation.id,
            reservation.sum()
        );

        // Handle cases where no swapping is needed:
        // - The target amount is zero
        // - The reservation already matches the total target amounts and each target amount
        //   can be selected from the reserved leaves
        let total_amount_sats = target_amounts.map(|ta| ta.total_sats()).unwrap_or(0);
        if (total_amount_sats == 0 || reservation.sum() == total_amount_sats)
            && select_helper::select_leaves_by_target_amounts(&reservation.leaves, target_amounts)
                .is_ok()
        {
            trace!("Selected leaves match requirements, no swap needed");
            return Ok(reservation);
        }

        // Swap the leaves to match the target amount.
        with_reserved_leaves(
            self,
            self.swap_leaves_internal(&reservation.leaves, target_amounts),
            &reservation,
        )
        .await?;
        trace!("Swapped leaves to match target amount");
        // Now the leaves should contain the exact amount.
        let reservation = self
            .reserve_fresh_leaves(target_amounts, true, purpose)
            .await?
            .ok_or(TreeServiceError::InsufficientFunds)?;
        trace!(
            "Selected leaves got reservation after swap: {:?} ({})",
            reservation.id,
            reservation.sum()
        );
        Ok(reservation)
    }

    async fn refresh_leaves(&self) -> Result<(), TreeServiceError> {
        // Prepare queries for coordinator and all operators and run them in parallel
        let coordinator_client = self.operator_pool.get_coordinator().client.clone();
        let operators: Vec<_> = self
            .operator_pool
            .get_non_coordinator_operators()
            .map(|op| (op.id, op.client.clone()))
            .collect();

        let coord_fut = self.query_nodes(&coordinator_client, false, None);
        let op_futs = operators
            .iter()
            .map(|(id, client)| async move { (*id, self.query_nodes(client, false, None).await) });

        let (coordinator_leaves_res, operator_results) = tokio::join!(coord_fut, join_all(op_futs));
        let coordinator_leaves = coordinator_leaves_res?;

        // Propagate any operator query error to preserve original behavior and
        // collect successful operator leaves for later comparison
        let mut operator_leaves_vec: Vec<Vec<TreeNode>> = Vec::new();
        for (id, res) in operator_results {
            match res {
                Ok(leaves) => operator_leaves_vec.push(leaves),
                Err(e) => {
                    error!("Failed to query operator {id}: {e:?}");
                    return Err(e);
                }
            }
        }

        let mut missing_operator_leaves_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();
        let mut ignored_leaves_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();

        // For each operator's leaves, compare against coordinator in the same way as before
        for (operator_id, operator_leaves) in
            operators.into_iter().zip(operator_leaves_vec.into_iter())
        {
            for leaf in &coordinator_leaves {
                match operator_leaves.iter().find(|l| l.id == leaf.id) {
                    Some(operator_leaf) => {
                        // TODO: move this logic to TreeNode method
                        if operator_leaf.status != leaf.status
                            || operator_leaf.signing_keyshare.public_key
                                != leaf.signing_keyshare.public_key
                            || operator_leaf.node_tx != leaf.node_tx
                            || operator_leaf.refund_tx != leaf.refund_tx
                        {
                            warn!(
                                "Ignoring leaf due to mismatch between coordinator and operator {}. Coordinator: {:?}, Operator: {:?}",
                                operator_id.0, leaf, operator_leaf
                            );
                            missing_operator_leaves_map.insert(leaf.id.clone(), leaf.clone());
                        }
                    }
                    None => {
                        warn!(
                            "Ignoring leaf due to missing from operator {}: {:?}",
                            operator_id.0, leaf.id
                        );
                        missing_operator_leaves_map.insert(leaf.id.clone(), leaf.clone());
                    }
                }
            }
        }

        for leaf in &coordinator_leaves {
            if leaf.status != TreeNodeStatus::Available {
                info!("Ignoring leaf {} due to status: {:?}", leaf.id, leaf.status);
                ignored_leaves_map.insert(leaf.id.clone(), leaf.clone());
                continue;
            }

            let our_node_pubkey = self.signer.get_public_key_for_node(&leaf.id).await?;
            let other_node_pubkey = leaf.signing_keyshare.public_key;
            let verifying_pubkey = leaf.verifying_public_key;

            let combined_pubkey = our_node_pubkey.combine(&other_node_pubkey).map_err(|_| {
                TreeServiceError::Generic("Failed to combine public keys".to_string())
            })?;

            if combined_pubkey != verifying_pubkey {
                warn!(
                    "Leaf {}'s verifying public key does not match the expected value",
                    leaf.id
                );
                ignored_leaves_map.insert(leaf.id.clone(), leaf.clone());
            }
        }

        let new_leaves = coordinator_leaves
            .into_iter()
            .filter(|leaf| {
                !missing_operator_leaves_map.contains_key(&leaf.id)
                    && !ignored_leaves_map.contains_key(&leaf.id)
            })
            .collect::<Vec<_>>();
        let missing_operator_leaves = missing_operator_leaves_map
            .values()
            .filter(|leaf_id| !ignored_leaves_map.contains_key(&leaf_id.id))
            .cloned()
            .collect::<Vec<_>>();
        let refreshed_leaves = self
            .check_renew_nodes(new_leaves, async |e| {
                // If this is a partial check timelock error, the extend node timelock failed
                // but we can still update the leaves that were refreshed
                if let ServiceError::PartialCheckTimelockError(ref nodes) = e
                    && let Err(e) = self.state.set_leaves(nodes, &missing_operator_leaves).await
                {
                    error!("Failed to set leaves: {e:?}");
                }
            })
            .await?;

        self.state
            .set_leaves(&refreshed_leaves, &missing_operator_leaves)
            .await?;

        Ok(())
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        let leaves = self.state.get_leaves().await?;
        Ok(leaves.balance())
    }
}

impl SynchronousTreeService {
    pub fn new(
        identity_pubkey: PublicKey,
        network: Network,
        operator_pool: Arc<OperatorPool>,
        state: Arc<dyn TreeStore>,
        timelock_manager: Arc<TimelockManager>,
        signer: Arc<dyn Signer>,
        swap_service: Arc<Swap>,
    ) -> Self {
        SynchronousTreeService {
            identity_pubkey,
            network,
            operator_pool,
            state,
            timelock_manager,
            signer,
            swap_service,
        }
    }

    async fn query_nodes_inner(
        &self,
        client: &SparkRpcClient,
        include_parents: bool,
        source: Option<Source>,
        paging: PagingFilter,
    ) -> Result<PagingResult<TreeNode>, TreeServiceError> {
        trace!(
            "Querying nodes with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let source = source.unwrap_or(Source::OwnerIdentityPubkey(
            self.identity_pubkey.serialize().to_vec(),
        ));
        let nodes = client
            .query_nodes(QueryNodesRequest {
                include_parents,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                network: self.network.to_proto_network().into(),
                source: Some(source),
                statuses: vec![],
            })
            .await?;
        Ok(PagingResult {
            items: nodes
                .nodes
                .into_values()
                .map(TreeNode::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    TreeServiceError::Generic(format!("Failed to deserialize leaves: {e:?}"))
                })?,
            next: paging.next_from_offset(nodes.offset),
        })
    }

    async fn query_nodes(
        &self,
        client: &SparkRpcClient,
        include_parents: bool,
        source: Option<Source>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let nodes = pager(
            |f| self.query_nodes_inner(client, include_parents, source.clone(), f),
            PagingFilter::default(),
        )
        .await?;
        Ok(nodes.items)
    }

    async fn check_renew_nodes<F>(
        &self,
        nodes: Vec<TreeNode>,
        error_fn: impl FnOnce(ServiceError) -> F,
    ) -> Result<Vec<TreeNode>, TreeServiceError>
    where
        F: Future<Output = ()>,
    {
        match self.timelock_manager.check_renew_nodes(nodes).await {
            Ok(nodes) => Ok(nodes),
            Err(e) => {
                error_fn(e.clone()).await;
                Err(TreeServiceError::Generic(format!(
                    "Failed to check time lock: {e:?}"
                )))
            }
        }
    }

    async fn reserve_fresh_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<Option<LeavesReservation>, TreeServiceError> {
        trace!("Reserving leaves for amounts: {target_amounts:?}, purpose: {purpose:?}");
        let reservation = self
            .state
            .reserve_leaves(target_amounts, exact_only, purpose)
            .await?;

        let new_leaves = self
            .check_renew_nodes(reservation.leaves, async |e| {
                // Cancel the reservation if the timelock check fails
                if let Err(e) = self.state.cancel_reservation(&reservation.id).await {
                    error!("Failed to cancel reservation: {e:?}");
                    return;
                }
                // If this is a partial check timelock error, the extend node timelock failed
                // but we can still update the leaves that were refreshed
                if let ServiceError::PartialCheckTimelockError(ref nodes) = e
                    && let Err(e) = self.state.add_leaves(nodes).await
                {
                    error!("Failed to add leaves: {e:?}");
                }
            })
            .await?;

        Ok(Some(LeavesReservation::new(new_leaves, reservation.id)))
    }

    async fn swap_leaves_internal(
        &self,
        leaves: &[TreeNode],
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if leaves.is_empty() {
            return Err(TreeServiceError::Generic("no leaves to swap".to_string()));
        }

        let target_amounts = target_amounts.map(|ta| match ta {
            TargetAmounts::AmountAndFee {
                amount_sats,
                fee_sats,
            } => {
                let mut amounts = vec![*amount_sats];
                if let Some(fee_sats) = fee_sats {
                    amounts.push(*fee_sats);
                }
                amounts
            }
            TargetAmounts::ExactDenominations { denominations } => denominations.clone(),
        });
        let claimed_nodes = self
            .swap_service
            .swap_leaves(leaves, target_amounts)
            .await?;

        let result_nodes = self.insert_leaves(claimed_nodes.clone()).await?;

        Ok(result_nodes)
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use macros::test_all;
    use uuid::Uuid;

    use super::*;
    use crate::tree::{
        SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus,
        select_helper::{find_exact_multiple_match, find_exact_single_match},
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // Helper function to create test leaves with specific values
    fn create_test_leaves(values: &[u64]) -> Vec<TreeNode> {
        values
            .iter()
            .map(|&value| TreeNode {
                id: TreeNodeId::generate(),
                tree_id: Uuid::now_v7().to_string(),
                value,
                parent_node_id: None,
                node_tx: Transaction {
                    version: Version::TWO,
                    lock_time: LockTime::ZERO,
                    input: vec![],
                    output: vec![],
                },
                refund_tx: None,
                direct_tx: None,
                direct_refund_tx: None,
                direct_from_cpfp_refund_tx: None,
                vout: 0,
                verifying_public_key: PublicKey::from_slice(&[2; 33]).unwrap(),
                owner_identity_public_key: PublicKey::from_slice(&[2; 33]).unwrap(),
                signing_keyshare: SigningKeyshare {
                    owner_identifiers: Vec::new(),
                    threshold: 0,
                    public_key: PublicKey::from_slice(&[2; 33]).unwrap(),
                },
                status: TreeNodeStatus::Available,
            })
            .collect()
    }

    #[test_all]
    fn test_find_exact_single_match() {
        let leaves = create_test_leaves(&[10000, 5000, 3000, 1000]);

        // Should find an exact match
        let result = find_exact_single_match(&leaves, 5000);
        assert!(result.is_some());
        assert_eq!(result.unwrap().value, 5000);

        // Should not find a match
        let result = find_exact_single_match(&leaves, 7000);
        assert!(result.is_none());
    }

    #[test_all]
    fn test_find_exact_multiple_match_simple_case() {
        let leaves = create_test_leaves(&[10000, 5000, 3000, 1000]);

        // Should find 5000 + 1000
        let result = find_exact_multiple_match(&leaves, 6000);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 6000);

        // Verify we're using the correct leaves (we know our implementation will
        // select 5000 + 1000 rather than 3000 + 3000 because leaves are processed in order)
        let values: Vec<u64> = selected.iter().map(|leaf| leaf.value).collect();
        assert!(values.contains(&5000));
        assert!(values.contains(&1000));
    }

    #[test_all]
    fn test_find_exact_multiple_match_complex_case() {
        let leaves = create_test_leaves(&[10000, 7000, 5000, 3000, 2000, 1000]);

        // Should find a combination adding up to 12000
        let result = find_exact_multiple_match(&leaves, 12000);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 12000);
    }

    #[test_all]
    fn test_find_exact_multiple_match_edge_cases() {
        // Empty leaves
        let leaves = Vec::<TreeNode>::new();
        assert!(find_exact_multiple_match(&leaves, 1000).is_none());

        // Zero target
        let leaves = create_test_leaves(&[1000, 500]);
        assert_eq!(find_exact_multiple_match(&leaves, 0).unwrap().len(), 0);

        // Impossible combination
        let leaves = create_test_leaves(&[10000, 5000, 3000]);
        assert!(find_exact_multiple_match(&leaves, 7000).is_none());

        // Target equals single leaf value
        let leaves = create_test_leaves(&[10000, 5000, 3000]);
        let result = find_exact_multiple_match(&leaves, 5000);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value, 5000);
    }

    #[test_all]
    fn test_find_exact_multiple_match_large_values() {
        // Test with larger values to ensure our algorithm scales properly
        let leaves =
            create_test_leaves(&[100_000_000, 50_000_000, 30_000_000, 10_000_000, 5_000_000]);

        // Should find a combination adding up to 65_000_000
        let result = find_exact_multiple_match(&leaves, 65_000_000);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 65_000_000);
    }
}
