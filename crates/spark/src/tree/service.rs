use std::collections::HashSet;
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            SparkRpcClient,
            spark::{QueryNodesRequest, query_nodes_request::Source},
        },
    },
    services::{Swap, TimelockManager, TransferService},
    signer::Signer,
    tree::{
        LeavesReservation, LeavesReservationId, TargetAmounts, TargetLeaves, TreeNodeId,
        TreeNodeStatus,
    },
    utils::paging::{PagingFilter, PagingResult, pager},
};

use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeServiceParams<S> {
    pub identity_pubkey: PublicKey,
    pub network: Network,
    pub operator_pool: Arc<OperatorPool<S>>,
    pub state: TreeState,
    pub timelock_manager: Arc<TimelockManager<S>>,
    pub signer: Arc<S>,
    pub swap_service: Swap<S>,
    pub transfer_service: Arc<TransferService<S>>,
}

pub struct TreeService<S> {
    identity_pubkey: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool<S>>,
    state: Mutex<TreeState>,
    timelock_manager: Arc<TimelockManager<S>>,
    signer: Arc<S>,
    swap_service: Swap<S>,
    transfer_service: Arc<TransferService<S>>,
    leaf_optimization_lock: Mutex<()>,
}

impl<S: Signer> TreeService<S> {
    pub fn new(params: TreeServiceParams<S>) -> Self {
        TreeService {
            identity_pubkey: params.identity_pubkey,
            network: params.network,
            operator_pool: params.operator_pool,
            state: Mutex::new(params.state),
            timelock_manager: params.timelock_manager,
            signer: params.signer,
            swap_service: params.swap_service,
            transfer_service: params.transfer_service,
            leaf_optimization_lock: Mutex::new(()),
        }
    }

    async fn query_nodes_inner(
        &self,
        client: &SparkRpcClient<S>,
        paging: PagingFilter,
    ) -> Result<PagingResult<TreeNode>, TreeServiceError> {
        trace!(
            "Querying nodes with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let nodes = client
            .query_nodes(QueryNodesRequest {
                include_parents: false,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                network: self.network.to_proto_network().into(),
                source: Some(Source::OwnerIdentityPubkey(
                    self.identity_pubkey.serialize().to_vec(),
                )),
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
        client: &SparkRpcClient<S>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let nodes = pager(
            |f| self.query_nodes_inner(client, f),
            PagingFilter::default(),
        )
        .await?;
        Ok(nodes)
    }

    /// Lists all leaves from the local cache.
    ///
    /// This method retrieves the current set of tree nodes stored in the local state
    /// without making any network calls. To update the cache with the latest data
    /// from the server, call [`refresh_leaves`] first.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<TreeNode>, TreeServiceError>` - A vector of tree nodes representing
    ///   the leaves in the local cache, or an error if the operation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeServiceError};
    /// use spark::signer::Signer;
    ///
    /// # async fn example(tree_service: &TreeService<impl Signer>) -> Result<(), TreeServiceError> {
    /// // First refresh to get the latest data
    /// tree_service.refresh_leaves().await?;
    ///
    /// // Then list the leaves
    /// let leaves = tree_service.list_leaves().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_leaves(&self) -> Result<Vec<TreeNode>, TreeServiceError> {
        Ok(self.state.lock().await.get_leaves())
    }

    /// Refreshes the tree state by fetching the latest leaves from the server.
    ///
    /// This method clears the current local cache of leaves and fetches all available
    /// leaves from the coordinator, storing them in the local state. It handles pagination
    /// internally and will continue fetching until all leaves have been retrieved.
    ///
    /// # Returns
    ///
    /// * `Result<(), TreeServiceError>` - Ok if the refresh was successful, or an error
    ///   if any part of the operation fails.
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * Communication with the server fails
    /// * Deserialization of leaf data fails
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeServiceError};
    /// use spark::signer::Signer;
    ///
    /// # async fn example(tree_service: &TreeService<impl Signer>) -> Result<(), TreeServiceError> {
    /// // Refresh the local cache with the latest leaves from the server
    /// tree_service.refresh_leaves().await?;
    ///
    /// // Now you can work with the updated leaves
    /// let leaves = tree_service.list_leaves().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn refresh_leaves(&self) -> Result<(), TreeServiceError> {
        let coordinator_leaves = self
            .query_nodes(&self.operator_pool.get_coordinator().client)
            .await?;

        let mut leaves_to_ignore: HashSet<TreeNodeId> = HashSet::new();

        // TODO: on js sdk, leaves missing from operators are not ignored when checking balance
        // TODO: we can optimize this by fetching leaves from all operators in parallel
        for operator in self.operator_pool.get_non_coordinator_operators() {
            let operator_leaves = self.query_nodes(&operator.client).await?;

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
                                operator.id, leaf, operator_leaf
                            );
                            leaves_to_ignore.insert(leaf.id.clone());
                        }
                    }
                    None => {
                        warn!(
                            "Ignoring leaf due to missing from operator {}: {:?}",
                            operator.id, leaf.id
                        );
                        leaves_to_ignore.insert(leaf.id.clone());
                    }
                }
            }
        }

        for leaf in &coordinator_leaves {
            let our_node_pubkey = self.signer.get_public_key_for_node(&leaf.id)?;
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
                leaves_to_ignore.insert(leaf.id.clone());
            }
        }

        let new_leaves = coordinator_leaves
            .into_iter()
            .filter(|leaf| !leaves_to_ignore.contains(&leaf.id))
            .collect::<Vec<_>>();

        let refreshed_leaves = self
            .timelock_manager
            .check_timelock_nodes(new_leaves)
            .await
            .map_err(|e| TreeServiceError::Generic(format!("Failed to check time lock: {e:?}")))?;

        {
            let mut state = self.state.lock().await;
            state.set_leaves(&refreshed_leaves);
        }

        self.optimize_leaves().await?;

        Ok(())
    }

    async fn optimize_leaves(&self) -> Result<(), TreeServiceError> {
        if let Ok(_guard) = self.leaf_optimization_lock.try_lock() {
            if !self.leaves_need_optimization().await {
                debug!("Leaves do not need optimization, skipping");
                return Ok(());
            }
            if let Some(reservation) = self.reserve_leaves(None, false).await? {
                debug!("Optimizing {} leaves", reservation.leaves.len());
                let optimized_leaves = self
                    .with_reserved_leaves(
                        self.swap_leaves_internal(&reservation.leaves, None),
                        &reservation,
                    )
                    .await?;
                trace!("Optimized leaves: {optimized_leaves:?}");
            }
        } else {
            debug!("Leaf optimization already in progress, skipping");
        }
        Ok(())
    }

    async fn leaves_need_optimization(&self) -> bool {
        let state = self.state.lock().await;
        let leaves = state.get_leaves();

        if leaves.len() <= 1 {
            return false;
        }

        let total_amount_sats = leaves.iter().map(|leaf| leaf.value).sum::<u64>();

        // Calculate the optimal number of leaves by counting set bits in binary representation
        // This is equivalent to the JavaScript algorithm that uses powers of 2
        let optimal_leaves_length = total_amount_sats.count_ones() as usize;

        leaves.len() > optimal_leaves_length * 5
    }

    pub async fn reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
    ) -> Result<Option<LeavesReservation>, TreeServiceError> {
        trace!("Reserving leaves for amounts: {target_amounts:?}");
        let reservation = {
            let mut state = self.state.lock().await;
            // Filter available leaves from the state
            let leaves: Vec<TreeNode> = state
                .get_leaves()
                .into_iter()
                .filter(|leaf| leaf.status == TreeNodeStatus::Available)
                .collect();
            // Select leaves that match the target amounts
            let target_leaves_res = self.select_leaves_by_amounts(&leaves, target_amounts);
            let selected = match target_leaves_res {
                Ok(target_leaves) => {
                    // Successfully selected target leaves
                    trace!("Successfully selected target leaves");
                    [
                        target_leaves.amount_leaves,
                        target_leaves.fee_leaves.unwrap_or_default(),
                    ]
                    .concat()
                }
                Err(_) if !exact_only => {
                    trace!("No exact match found, selecting leaves by minimum amount");
                    let target_amount_sat = target_amounts.map_or(0, |ta| ta.total_sats());
                    let Some(selected) =
                        self.select_leaves_by_minimum_amount(&leaves, target_amount_sat)?
                    else {
                        return Ok(None);
                    };
                    selected
                }
                Err(e) => {
                    error!("Failed to select target leaves: {e:?}");
                    return Ok(None);
                }
            };

            let reservation_id = state.reserve_leaves(&selected)?;
            LeavesReservation::new(selected, reservation_id)
        };

        let new_leaves = self
            .timelock_manager
            .check_timelock_nodes(reservation.leaves)
            .await
            .map_err(|e| TreeServiceError::Generic(format!("Failed to check time lock: {e:?}")))?;
        Ok(Some(LeavesReservation::new(new_leaves, reservation.id)))
    }

    pub async fn cancel_reservation(&self, id: LeavesReservationId) {
        let mut state = self.state.lock().await;
        state.cancel_reservation(id);
    }

    pub async fn finalize_reservation(&self, id: LeavesReservationId) {
        let mut state = self.state.lock().await;
        state.finalize_reservation(id);
    }

    pub async fn insert_leaves(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let result_nodes = self
            .timelock_manager
            .check_timelock_nodes(leaves)
            .await
            .map_err(|e| TreeServiceError::Generic(format!("Failed to check time lock: {e:?}")))?;
        let mut state = self.state.lock().await;
        state.add_leaves(&result_nodes);
        Ok(result_nodes)
    }

    /// Selects leaves from the tree that sum up to exactly the target amounts.
    /// If such a combination of leaves does not exist, it performs a swap to get a set of leaves matching the target amounts.
    /// If no leaves can be selected, returns an error
    pub async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!("Selecting leaves for target amounts: {target_amounts:?}");
        let reservation = self.reserve_leaves(target_amounts, false).await?;
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
            && self
                .select_leaves_by_amounts(&reservation.leaves, target_amounts)
                .is_ok()
        {
            trace!("Selected leaves match requirements, no swap needed");
            return Ok(reservation);
        }

        // Swap the leaves to match the target amount.
        self.with_reserved_leaves(
            self.swap_leaves_internal(&reservation.leaves, target_amounts),
            &reservation,
        )
        .await?;
        trace!("Swapped leaves to match target amount");
        // Now the leaves should contain the exact amount.
        let reservation = self
            .reserve_leaves(target_amounts, true)
            .await?
            .ok_or(TreeServiceError::InsufficientFunds)?;
        trace!(
            "Selected leaves got reservation after swap: {:?} ({})",
            reservation.id,
            reservation.sum()
        );
        Ok(reservation)
    }

    /// Selects leaves from the tree that match the target amounts.
    /// If no target amounts are specified, it returns all leaves.
    /// If the target amounts cannot be matched exactly, it returns an error.
    pub fn select_leaves_by_amounts(
        &self,
        leaves: &[TreeNode],
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<TargetLeaves, TreeServiceError> {
        let mut remaining_leaves = leaves.to_vec();

        // If no target amounts are specified, return all remaining leaves
        let Some(target_amounts) = target_amounts else {
            trace!("No target amounts specified, returning all remaining leaves");
            return Ok(TargetLeaves::new(remaining_leaves, None));
        };

        // Select leaves that match the target amount_sats
        let amount_leaves = self
            .select_leaves_by_amount(&remaining_leaves, target_amounts.amount_sats)?
            .ok_or(TreeServiceError::UnselectableAmount)?;

        let fee_leaves = match target_amounts.fee_sats {
            Some(fee_sats) => {
                // Remove the amount_leaves from remaining_leaves to avoid double spending
                remaining_leaves.retain(|leaf| {
                    !amount_leaves
                        .iter()
                        .any(|amount_leaf| amount_leaf.id == leaf.id)
                });
                // Select leaves that match the fee_sats from the remaining leaves
                Some(
                    self.select_leaves_by_amount(&remaining_leaves, fee_sats)?
                        .ok_or(TreeServiceError::UnselectableAmount)?,
                )
            }
            None => None,
        };

        Ok(TargetLeaves::new(amount_leaves, fee_leaves))
    }

    /// Selects leaves from the tree that sum up to exactly the target amount.
    /// If such a combination of leaves does not exist, it returns `None`.
    fn select_leaves_by_amount(
        &self,
        leaves: &[TreeNode],
        target_amount_sat: u64,
    ) -> Result<Option<Vec<TreeNode>>, TreeServiceError> {
        if target_amount_sat == 0 {
            return Err(TreeServiceError::InvalidAmount);
        }

        if leaves.iter().map(|leaf| leaf.value).sum::<u64>() < target_amount_sat {
            return Err(TreeServiceError::InsufficientFunds);
        }

        // Try to find a single leaf that matches the exact amount
        if let Some(leaf) = find_exact_single_match(leaves, target_amount_sat) {
            return Ok(Some(vec![leaf]));
        }

        // Try to find a set of leaves that sum exactly to the target amount
        if let Some(selected_leaves) = find_exact_multiple_match(leaves, target_amount_sat) {
            return Ok(Some(selected_leaves));
        }

        Ok(None)
    }

    /// Selects leaves from the tree that sum up to at least the target amount.
    fn select_leaves_by_minimum_amount(
        &self,
        leaves: &[TreeNode],
        target_amount_sat: u64,
    ) -> Result<Option<Vec<TreeNode>>, TreeServiceError> {
        if target_amount_sat == 0 {
            return Err(TreeServiceError::InvalidAmount);
        }

        let mut result = Vec::new();
        let mut sum = 0;
        for leaf in leaves {
            sum += leaf.value;
            result.push(leaf.clone());
            if sum >= target_amount_sat {
                break;
            }
        }

        if sum < target_amount_sat {
            return Ok(None);
        }

        Ok(Some(result))
    }

    // TODO: right now, this looks tighly coupled to claiming a deposit.
    //  We should either move this to the deposit service or make it more general.
    //  If made more general, should also be moved to timelock manager.
    pub async fn collect_leaves(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        let node_ids: Vec<_> = nodes.iter().map(|n| n.id.clone()).collect();
        let mut resulting_nodes = Vec::new();
        for node in nodes.into_iter() {
            if node.status != TreeNodeStatus::Available {
                warn!("Leaf is not available: {node:?}");
                // TODO: Handle other statuses appropriately.
                resulting_nodes.push(node);
                continue;
            }

            let nodes = self
                .timelock_manager
                .extend_time_lock(&node)
                .await
                .map_err(|e| {
                    TreeServiceError::Generic(format!("Failed to extend time lock: {e:?}"))
                })?;

            for n in nodes {
                if n.status != TreeNodeStatus::Available {
                    warn!("Leaf resulting from extend_time_lock is not available: {n:?}",);
                    // TODO: Handle other statuses appropriately.
                    resulting_nodes.push(n);
                    continue;
                }

                let transfer = self
                    .timelock_manager
                    .transfer_leaves_to_self(vec![n])
                    .await
                    .map_err(|e| {
                        TreeServiceError::Generic(format!(
                            "Failed to transfer leaves to self: {e:?}"
                        ))
                    })?;
                resulting_nodes.extend(transfer.into_iter());
            }
        }

        let mut state = self.state.lock().await;
        state.remove_leaves(&node_ids);
        state.add_leaves(&resulting_nodes);
        Ok(resulting_nodes)
    }

    /// Returns the total balance of all available leaves in the tree.
    ///
    /// This method calculates the sum of all leaf values that have a status of
    /// `TreeNodeStatus::Available`. It first retrieves all leaves from the local cache
    /// and filters out any that are not available before calculating the total.
    ///
    /// # Returns
    ///
    /// * `Result<u64, TreeServiceError>` - The total balance in satoshis if successful,
    ///   or an error if the operation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeServiceError};
    /// use spark::signer::Signer;
    ///
    /// # async fn example(tree_service: &TreeService<impl Signer>) -> Result<(), TreeServiceError> {
    /// // Ensure the cache is up to date
    /// tree_service.refresh_leaves().await?;
    ///
    /// // Get the available balance
    /// let balance = tree_service.get_available_balance().await?;
    /// println!("Available balance: {} sats", balance);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        Ok(self
            .list_leaves()
            .await?
            .into_iter()
            .filter(|leaf| leaf.status == TreeNodeStatus::Available)
            .map(|leaf| leaf.value)
            .sum::<u64>())
    }

    pub async fn with_reserved_leaves<F, R, E>(
        &self,
        f: F,
        leaves: &LeavesReservation,
    ) -> Result<R, E>
    where
        F: Future<Output = Result<R, E>>,
    {
        match f.await {
            Ok(r) => {
                self.finalize_reservation(leaves.id.clone()).await;
                Ok(r)
            }
            Err(e) => {
                self.cancel_reservation(leaves.id.clone()).await;
                Err(e)
            }
        }
    }

    pub async fn swap_leaves_internal(
        &self,
        leaves: &[TreeNode],
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if leaves.is_empty() {
            return Err(TreeServiceError::Generic("no leaves to swap".to_string()));
        }

        let target_amounts = target_amounts.map(|ta| ta.to_vec());
        let transfer = self
            .swap_service
            .swap_leaves(leaves, target_amounts)
            .await?;
        let leaves = self.claim_and_insert_transfer(&transfer).await?;
        Ok(leaves)
    }

    /// Claims a transfer and inserts the resulting leaves into the tree
    pub async fn claim_and_insert_transfer(
        &self,
        transfer: &crate::services::Transfer,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        use crate::services::ServiceError;

        trace!("Claiming transfer with id: {}", transfer.id);
        let claimed_nodes = self
            .transfer_service
            .claim_transfer(transfer, None)
            .await
            .map_err(|e: ServiceError| {
                TreeServiceError::Generic(format!("Failed to claim transfer: {e:?}"))
            })?;

        trace!("Inserting claimed leaves after claiming transfer");
        let result_nodes = self.insert_leaves(claimed_nodes.clone()).await?;

        Ok(result_nodes)
    }
}

fn find_exact_single_match(leaves: &[TreeNode], target_amount_sat: u64) -> Option<TreeNode> {
    leaves
        .iter()
        .find(|leaf| leaf.value == target_amount_sat)
        .cloned()
}

fn find_exact_multiple_match(leaves: &[TreeNode], target_amount_sat: u64) -> Option<Vec<TreeNode>> {
    use std::collections::HashMap;

    // Early return if target is 0 or if there are no leaves
    if target_amount_sat == 0 {
        return Some(Vec::new());
    }
    if leaves.is_empty() {
        return None;
    }

    // Sort leaves by value in descending order, as we want to use larger leaves first.
    // This avoids potentially consuming smaller leaves that could be used later for
    // smaller targets, like paying fees.
    let mut sorted_leaves = leaves.to_vec();
    sorted_leaves.sort_by(|a, b| b.value.cmp(&a.value));

    // Use dynamic programming with HashMap for space efficiency
    // dp[amount] = (leaf_idx, prev_amount) represents that we can achieve 'amount'
    // by using leaf at leaf_idx and then achieve prev_amount
    let mut dp: HashMap<u64, (usize, u64)> = HashMap::new();
    dp.insert(0, (usize::MAX, 0)); // Special marker for zero sum

    // Fill dp table
    for (leaf_idx, leaf) in sorted_leaves.iter().enumerate() {
        // Consider all amounts we can currently achieve
        let current_amounts: Vec<u64> = dp.keys().cloned().collect();

        for &current_amount in &current_amounts {
            let new_amount = current_amount + leaf.value;

            // If this new amount doesn't exceed our target and we haven't found a way to achieve it yet
            if new_amount <= target_amount_sat && !dp.contains_key(&new_amount) {
                dp.insert(new_amount, (leaf_idx, current_amount));
            }
        }

        // Early exit if we've found our target
        if dp.contains_key(&target_amount_sat) {
            break;
        }
    }

    // If target amount cannot be reached
    if !dp.contains_key(&target_amount_sat) {
        return None;
    }

    // Reconstruct the solution by backtracking through the dp table
    let mut result = Vec::new();
    let mut current_amount = target_amount_sat;

    while current_amount > 0 {
        let (leaf_idx, prev_amount) = *dp.get(&current_amount).unwrap();
        if leaf_idx == usize::MAX {
            break; // Reached the special zero marker
        }
        result.push(sorted_leaves[leaf_idx].clone());
        current_amount = prev_amount;
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use uuid::Uuid;

    use super::*;
    use crate::tree::{SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus};

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

    #[test]
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

    #[test]
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

    #[test]
    fn test_find_exact_multiple_match_complex_case() {
        let leaves = create_test_leaves(&[10000, 7000, 5000, 3000, 2000, 1000]);

        // Should find a combination adding up to 12000
        let result = find_exact_multiple_match(&leaves, 12000);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 12000);
    }

    #[test]
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

    #[test]
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
