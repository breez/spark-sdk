use futures::future::join_all;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bitcoin::secp256k1::PublicKey;
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, trace, warn};
use web_time::Instant;

/// Maximum time to wait for pending balance before giving up.
/// This prevents infinite hangs if the system gets into a bad state.
const MAX_WAIT_FOR_PENDING_DURATION: Duration = Duration::from_secs(60);

use crate::tree::{Leaves, ReservationPurpose, ReserveResult, TreeNodeStatus};
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
        select_helper,
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
    /// If no leaves can be selected, returns an error.
    ///
    /// Uses notification-based waiting: if balance is insufficient but pending
    /// balance from in-flight swaps would help, waits for balance changes
    /// instead of failing immediately.
    async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        debug!(
            "select_leaves starting | target_amounts={:?} purpose={:?}",
            target_amounts.map(|t| t.total_sats()),
            purpose
        );
        trace!("Selecting leaves for target amounts: {target_amounts:?}, purpose: {purpose:?}");

        let mut balance_rx = self.state.subscribe_balance_changes();
        let wait_start = web_time::Instant::now();
        let mut wait_count = 0u32;

        loop {
            let reserve_result = self
                .state
                .try_reserve_leaves(target_amounts, false, purpose)
                .await?;

            // Handle non-success cases first with early continue/return
            let reservation = match reserve_result {
                ReserveResult::InsufficientFunds => {
                    return Err(TreeServiceError::InsufficientFunds);
                }
                ReserveResult::WaitForPending {
                    needed,
                    available,
                    pending,
                } => {
                    self.wait_for_pending_balance(
                        &mut balance_rx,
                        &wait_start,
                        &mut wait_count,
                        needed,
                        available,
                        pending,
                    )
                    .await?;
                    continue;
                }
                ReserveResult::Success(r) => r,
            };

            trace!(
                "Selected leaves got reservation: {:?} ({})",
                reservation.id,
                reservation.sum()
            );

            let reservation = self.renew_reservation_timelocks(reservation).await?;

            // Check if swap is needed
            if self.reservation_matches_target(&reservation, target_amounts) {
                trace!("Selected leaves match requirements, no swap needed");
                return Ok(reservation);
            }

            // Perform swap and update reservation
            return self
                .perform_swap_and_update_reservation(reservation, target_amounts)
                .await;
        }
    }

    async fn refresh_leaves(&self) -> Result<(), TreeServiceError> {
        let start = Instant::now();
        debug!("refresh_leaves starting");
        debug!("refresh_leaves | calling operators to refresh leaves");

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

        debug!(
            "refresh_leaves completed | new_leaf_count={} elapsed_ms={}",
            refreshed_leaves.len(),
            start.elapsed().as_millis()
        );
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

    /// Checks if the reservation already matches the target amounts without needing a swap.
    fn reservation_matches_target(
        &self,
        reservation: &LeavesReservation,
        target_amounts: Option<&TargetAmounts>,
    ) -> bool {
        let total_amount_sats = target_amounts.map(|ta| ta.total_sats()).unwrap_or(0);
        (total_amount_sats == 0 || reservation.sum() == total_amount_sats)
            && select_helper::select_leaves_by_target_amounts(&reservation.leaves, target_amounts)
                .is_ok()
    }

    /// Waits for pending balance to become available.
    async fn wait_for_pending_balance(
        &self,
        balance_rx: &mut tokio::sync::watch::Receiver<u64>,
        wait_start: &web_time::Instant,
        wait_count: &mut u32,
        needed: u64,
        available: u64,
        pending: u64,
    ) -> Result<(), TreeServiceError> {
        *wait_count += 1;
        let elapsed = wait_start.elapsed();

        if elapsed > MAX_WAIT_FOR_PENDING_DURATION {
            warn!(
                "Timeout waiting for pending balance after {:?} ({} attempts): need={needed}, available={available}, pending={pending}",
                elapsed, wait_count
            );
            return Err(TreeServiceError::Generic(format!(
                "Timeout waiting for pending balance after {:?}",
                elapsed
            )));
        }

        trace!(
            "Waiting for pending balance (attempt {}, elapsed {:?}): need={needed}, available={available}, pending={pending}",
            wait_count, elapsed
        );

        let wait_timeout = Duration::from_secs(5);
        match tokio::time::timeout(wait_timeout, balance_rx.changed()).await {
            Ok(Ok(())) => {
                trace!("Balance change notification received, retrying");
            }
            Ok(Err(_)) => {
                return Err(TreeServiceError::Generic("Store closed".into()));
            }
            Err(_) => {
                trace!("Wait timeout after {:?}, retrying anyway", wait_timeout);
            }
        }
        Ok(())
    }

    /// Performs a swap and updates the reservation with the new leaves.
    async fn perform_swap_and_update_reservation(
        &self,
        reservation: LeavesReservation,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!("Swapping leaves to match target amount");

        let swap_result = self
            .swap_leaves_internal(&reservation.leaves, target_amounts)
            .await;

        let new_leaves = match swap_result {
            Ok(leaves) => leaves,
            Err(e) => {
                // Swap failed - cancel the reservation to release the permit
                if let Err(cancel_err) = self.state.cancel_reservation(&reservation.id).await {
                    error!("Failed to cancel reservation after swap error: {cancel_err:?}");
                }
                return Err(e);
            }
        };

        trace!(
            "Swapped leaves to match target amount, got {} new leaves",
            new_leaves.len()
        );

        // Select the exact leaves that match the target amounts
        let target_leaves =
            select_helper::select_leaves_by_target_amounts(&new_leaves, target_amounts)?;
        let reserved_leaves = [
            target_leaves.amount_leaves,
            target_leaves.fee_leaves.unwrap_or_default(),
        ]
        .concat();

        // Change leaves are the remaining leaves after selection
        let reserved_ids: std::collections::HashSet<_> =
            reserved_leaves.iter().map(|l| &l.id).collect();
        let change_leaves: Vec<_> = new_leaves
            .iter()
            .filter(|l| !reserved_ids.contains(&l.id))
            .cloned()
            .collect();

        // Update the existing reservation with the selected leaves.
        // Change leaves are added to the pool atomically, preventing
        // race conditions where another request could grab them.
        let update_result = self
            .state
            .update_reservation(&reservation.id, &reserved_leaves, &change_leaves)
            .await;

        match update_result {
            Ok(final_reservation) => {
                trace!(
                    "Selected leaves got reservation after swap: {:?} ({})",
                    final_reservation.id,
                    final_reservation.sum()
                );
                Ok(final_reservation)
            }
            Err(e) => {
                // Update failed - finalize the reservation to release the permit.
                // We use finalize (not cancel) because the OLD leaves were
                // consumed by the swap and no longer exist.
                // Pass the new swap output to preserve them in the pool.
                error!("Failed to update reservation after swap: {e:?}, finalizing");
                if let Err(finalize_err) = self
                    .state
                    .finalize_reservation(&reservation.id, Some(&new_leaves))
                    .await
                {
                    error!("Failed to finalize reservation after update error: {finalize_err:?}");
                }
                Err(e)
            }
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

    /// Renew timelocks for reserved leaves and handle partial failures.
    async fn renew_reservation_timelocks(
        &self,
        reservation: LeavesReservation,
    ) -> Result<LeavesReservation, TreeServiceError> {
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

        Ok(LeavesReservation::new(new_leaves, reservation.id))
    }

    /// Performs a swap operation and returns the new leaves.
    ///
    /// Note: This method does NOT add the new leaves to the store. The caller
    /// is responsible for adding them (e.g., via `update_reservation`
    /// which adds and reserves atomically to avoid race conditions).
    async fn swap_leaves_internal(
        &self,
        leaves: &[TreeNode],
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let start = Instant::now();
        debug!(
            "swap_leaves_internal starting | leaf_count={}",
            leaves.len()
        );

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

        // Check/renew timelocks on the new leaves, but don't add to store yet.
        // The caller will add them atomically with the reservation update.
        let result_nodes = self
            .check_renew_nodes(claimed_nodes, async |_| {
                // On partial failure, we can't do much here since the leaves
                // aren't in the store yet. The caller will handle the error.
            })
            .await?;

        debug!(
            "swap_leaves_internal completed | result_leaf_count={} elapsed_ms={}",
            result_nodes.len(),
            start.elapsed().as_millis()
        );
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
                    version: Version::non_standard(3),
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
        let leaves = create_test_leaves(&[8192, 4096, 2048, 1024]);

        // Should find an exact match
        let result = find_exact_single_match(&leaves, 4096);
        assert!(result.is_some());
        assert_eq!(result.unwrap().value, 4096);

        // Should not find a match
        let result = find_exact_single_match(&leaves, 7000);
        assert!(result.is_none());
    }

    #[test_all]
    fn test_find_exact_multiple_match_simple_case() {
        let leaves = create_test_leaves(&[8192, 4096, 2048, 1024]);

        // Should find 4096 + 1024 = 5120
        let result = find_exact_multiple_match(&leaves, 5120);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 5120);

        // Verify we're using the correct leaves (greedy picks largest first)
        let values: Vec<u64> = selected.iter().map(|leaf| leaf.value).collect();
        assert!(values.contains(&4096));
        assert!(values.contains(&1024));
    }

    #[test_all]
    fn test_find_exact_multiple_match_complex_case() {
        let leaves = create_test_leaves(&[16384, 8192, 4096, 2048, 1024, 512]);

        // Should find a combination adding up to 12288 (8192 + 4096)
        let result = find_exact_multiple_match(&leaves, 12288);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 12288);
    }

    #[test_all]
    fn test_find_exact_multiple_match_edge_cases() {
        // Empty leaves
        let leaves = Vec::<TreeNode>::new();
        assert!(find_exact_multiple_match(&leaves, 1024).is_none());

        // Zero target
        let leaves = create_test_leaves(&[1024, 512]);
        assert_eq!(find_exact_multiple_match(&leaves, 0).unwrap().len(), 0);

        // Impossible combination (greedy picks 8192, remaining=1808, can't make it)
        let leaves = create_test_leaves(&[8192, 4096, 2048]);
        assert!(find_exact_multiple_match(&leaves, 10000).is_none());

        // Target equals single leaf value
        let leaves = create_test_leaves(&[8192, 4096, 2048]);
        let result = find_exact_multiple_match(&leaves, 4096);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value, 4096);
    }

    #[test_all]
    fn test_find_exact_multiple_match_large_values() {
        // Test with larger power-of-two values to ensure our algorithm scales properly
        let leaves = create_test_leaves(&[
            134_217_728, // 2^27
            67_108_864,  // 2^26
            33_554_432,  // 2^25
            16_777_216,  // 2^24
            8_388_608,   // 2^23
        ]);

        // Should find a combination adding up to 100_663_296 (2^26 + 2^25)
        let result = find_exact_multiple_match(&leaves, 100_663_296);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 100_663_296);
    }

    #[test_all]
    fn test_greedy_with_non_power_of_two_success() {
        // [3000, 2048, 1024] targeting 3072
        // Pass 1: picks 3000, remaining=72, can't find → fails
        // Pass 2: filters to [2048, 1024], picks 2048, remaining=1024, picks 1024 → success!
        let leaves = create_test_leaves(&[3000, 2048, 1024]);

        let result = find_exact_multiple_match(&leaves, 3072);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 3072);

        // Verify we selected the power-of-two leaves, not the odd one
        let values: Vec<u64> = selected.iter().map(|leaf| leaf.value).collect();
        assert!(values.contains(&2048));
        assert!(values.contains(&1024));
        assert!(!values.contains(&3000));
    }

    #[test_all]
    fn test_greedy_with_non_power_of_two_failure() {
        // [3000, 2048, 1024] targeting 3080
        // Pass 1: picks 3000, remaining=80, can't find → fails
        // Pass 2: filters to [2048, 1024], picks 2048, remaining=1032, can't find → returns None
        let leaves = create_test_leaves(&[3000, 2048, 1024]);

        let result = find_exact_multiple_match(&leaves, 3080);
        assert!(result.is_none());
    }
}
