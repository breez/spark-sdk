use std::{collections::HashSet, sync::Arc};

use bitcoin::secp256k1::PublicKey;
use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            SparkRpcClient,
            spark::{QueryNodesRequest, query_nodes_request::Source},
        },
    },
    services::{PagingFilter, PagingResult, TimelockManager},
    signer::Signer,
    tree::{TreeNodeId, TreeNodeStatus},
};

use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeService<S> {
    identity_pubkey: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool<S>>,
    state: Mutex<TreeState>,
    timelock_manager: Arc<TimelockManager<S>>,
    signer: S,
}

impl<S: Signer> TreeService<S> {
    pub fn new(
        identity_pubkey: PublicKey,
        network: Network,
        operator_pool: Arc<OperatorPool<S>>,
        state: TreeState,
        timelock_manager: Arc<TimelockManager<S>>,
        signer: S,
    ) -> Self {
        TreeService {
            identity_pubkey,
            network,
            operator_pool,
            state: Mutex::new(state),
            timelock_manager,
            signer,
        }
    }

    // TODO: move this to a middle layer where we can handle paging for all queries where it makes sense
    async fn fetch_all_leaves_using_client(
        &self,
        client: &SparkRpcClient<S>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let mut paging = PagingFilter::default();
        let mut all_leaves = Vec::new();
        loop {
            let leaves = self.fetch_leaves_using_client(client, &paging).await?;
            if leaves.items.is_empty() {
                break;
            }

            all_leaves.extend(leaves.items);

            match leaves.next {
                None => break,
                Some(next) => paging = next,
            }
        }
        Ok(all_leaves)
    }

    // TODO: move this to a middle layer where we can handle paging for all queries where it makes sense
    async fn fetch_leaves_using_client(
        &self,
        client: &SparkRpcClient<S>,
        paging: &PagingFilter,
    ) -> Result<PagingResult<TreeNode>, TreeServiceError> {
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
            .fetch_all_leaves_using_client(&self.operator_pool.get_coordinator().client)
            .await?;

        let mut leaves_to_ignore: HashSet<TreeNodeId> = HashSet::new();

        // TODO: on js sdk, leaves missing from operators are not ignored when checking balance
        // TODO: we can optimize this by fetching leaves from all operators in parallel
        for operator in self.operator_pool.get_non_coordinator_operators() {
            let operator_leaves = self.fetch_all_leaves_using_client(&operator.client).await?;

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

        let mut state = self.state.lock().await;
        state.clear_leaves();
        state.add_leaves(&new_leaves);

        Ok(())
    }

    /// Selects leaves from the tree that sum up to exactly the target amount.
    /// If such a combination of leaves does not exist, it returns `None`.
    pub async fn select_leaves_by_amount(
        &self,
        target_amount_sat: u64,
    ) -> Result<Option<Vec<TreeNode>>, TreeServiceError> {
        if target_amount_sat == 0 {
            return Err(TreeServiceError::InvalidAmount);
        }

        let mut leaves = self.list_leaves().await?;

        // Only consider leaves that are available.
        leaves.retain(|leaf| leaf.status == TreeNodeStatus::Available);

        if leaves.iter().map(|leaf| leaf.value).sum::<u64>() < target_amount_sat {
            return Err(TreeServiceError::InsufficientFunds);
        }

        // Try to find a single leaf that matches the exact amount
        if let Some(leaf) = find_exact_single_match(&leaves, target_amount_sat) {
            return Ok(Some(vec![leaf]));
        }

        // Try to find a set of leaves that sum exactly to the target amount
        if let Some(selected_leaves) = find_exact_multiple_match(&leaves, target_amount_sat) {
            return Ok(Some(selected_leaves));
        }

        Ok(None)
    }

    /// Selects leaves from the tree that sum up to at least the target amount.
    pub async fn select_leaves_by_minimum_amount(
        &self,
        target_amount_sat: u64,
    ) -> Result<Option<Vec<TreeNode>>, TreeServiceError> {
        if target_amount_sat == 0 {
            return Err(TreeServiceError::InvalidAmount);
        }

        let mut leaves = self.list_leaves().await?;

        // Only consider leaves that are available.
        leaves.retain(|leaf| leaf.status == TreeNodeStatus::Available);

        // Sort leaves by value in ascending order, to prefer spending smaller leaves first.
        leaves.sort_by(|a, b| a.value.cmp(&b.value));

        let mut result = Vec::new();
        let mut sum = 0;
        for leaf in leaves {
            sum += leaf.value;
            result.push(leaf);
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

        // TODO: add/remove nodes to/from the tree state as needed.
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

    // Use dynamic programming with HashMap for space efficiency
    // dp[amount] = (leaf_idx, prev_amount) represents that we can achieve 'amount'
    // by using leaf at leaf_idx and then achieve prev_amount
    let mut dp: HashMap<u64, (usize, u64)> = HashMap::new();
    dp.insert(0, (usize::MAX, 0)); // Special marker for zero sum

    // Fill dp table
    for (leaf_idx, leaf) in leaves.iter().enumerate() {
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

        result.push(leaves[leaf_idx].clone());
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
