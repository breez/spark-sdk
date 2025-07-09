use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    Network,
    operator::rpc::{
        SparkRpcClient,
        spark::{QueryNodesRequest, query_nodes_request::Source},
    },
    services::{PagingFilter, PagingResult, TimelockManager, TransferService},
    signer::Signer,
    tree::TreeNodeStatus,
};

use super::{TreeNode, error::TreeServiceError, state::TreeState};

pub struct TreeService<S: Signer> {
    client: Arc<SparkRpcClient<S>>,
    identity_pubkey: PublicKey,
    network: Network,
    state: Mutex<TreeState>,
    timelock_manager: TimelockManager<S>,
    transfer_service: TransferService<S>,
}

impl<S: Signer> TreeService<S> {
    pub fn new(
        client: Arc<SparkRpcClient<S>>,
        identity_pubkey: PublicKey,
        network: Network,
        state: TreeState,
        timelock_manager: TimelockManager<S>,
        transfer_service: TransferService<S>,
    ) -> Self {
        TreeService {
            client,
            identity_pubkey,
            network,
            state: Mutex::new(state),
            timelock_manager,
            transfer_service,
        }
    }

    async fn fetch_leaves(
        &self,
        paging: &PagingFilter,
    ) -> Result<PagingResult<TreeNode>, TreeServiceError> {
        let nodes = self
            .client
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
                .into_iter()
                .map(|(_, node)| TreeNode::try_from(node))
                .collect::<Result<Vec<_>, _>>()?,
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
        let mut paging = PagingFilter::default();
        let mut new_leaves = Vec::new();
        loop {
            let leaves = self.fetch_leaves(&paging).await?;
            if leaves.items.is_empty() {
                break;
            }

            new_leaves.extend(leaves.items);

            match leaves.next {
                None => break,
                Some(next) => paging = next,
            }
        }

        let mut state = self.state.lock().await;
        state.clear_leaves();
        state.add_leaves(&new_leaves);

        Ok(())
    }

    /// Selects leaves from the tree that sum up to the target amount.
    /// If necessary, performs swap to get set of leaves matching target amount.
    pub async fn select_leaves_by_amount(
        &self,
        target_amount_sat: u64,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if target_amount_sat == 0 {
            return Err(TreeServiceError::IllegalAmount);
        }

        let mut amount = 0;
        let mut nodes = vec![];
        let mut leaves = self.list_leaves().await?;
        leaves.retain(|leaf| leaf.status == TreeNodeStatus::Available);
        leaves.sort_by(|a, b| b.value.cmp(&a.value));

        let mut aggregated_amount: u64 = 0;
        for leaf in leaves {
            aggregated_amount += leaf.value;
            if target_amount_sat.saturating_sub(amount) >= leaf.value {
                amount += leaf.value;
                nodes.push(leaf);
            }
        }
        if amount < target_amount_sat {
            match aggregated_amount > target_amount_sat {
                true => return Err(TreeServiceError::UnselectableAmount),
                false => return Err(TreeServiceError::InsufficientFunds),
            }
        }
        // TODO: if necessary, perform swap to get set of leaves matching target amount

        Ok(nodes)
    }

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

            let nodes = self.timelock_manager.extend_time_lock(&node).await?;

            for n in nodes {
                if n.status != TreeNodeStatus::Available {
                    warn!("Leaf resulting from extend_time_lock is not available: {n:?}",);
                    // TODO: Handle other statuses appropriately.
                    resulting_nodes.push(n);
                    continue;
                }

                let transfer = self
                    .transfer_service
                    .transfer_leaves_to_self(vec![n])
                    .await?;
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
