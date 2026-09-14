use spark::services::{ClaimTransferConfig, Transfer, TransferService};
use spark::tree::{LeavesReservationId, TreeNode, TreeNodeId, TreeServiceError, TreeStore};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Which id derives a pool leaf's signing key. A deposit-tree leaf signs under the
/// leaf id the SSP picked, not the node id the operators assigned, until the SSP
/// claims the leaf from a transfer.
#[async_trait::async_trait]
pub trait LeafSigningKeys: Send + Sync {
    /// The id `node_id`'s signing key derives from, or `None` when that is
    /// `node_id` itself.
    async fn get_signing_leaf_id(&self, node_id: &str) -> Result<Option<String>, String>;

    async fn mark_signing_under_node_id(&self, node_ids: &[String]) -> Result<(), String>;
}

/// Claims a transfer made to the SSP and takes its leaves into the pool.
///
/// Every claim goes through here, because claiming is what moves a leaf onto the
/// key derived from its node id: a leaf the SSP had fronted from a deposit tree
/// stops signing under the deposit leaf id the moment it comes back. Recording
/// that before the leaves reach the pool is what keeps the two in step, since a
/// spend that read the stale id would sign with a key the operators reject and
/// the leaf would be stuck.
pub async fn claim_into_pool(
    transfer_service: &TransferService,
    signing_keys: &dyn LeafSigningKeys,
    tree_store: &dyn TreeStore,
    transfer: &Transfer,
) -> Result<Vec<TreeNode>, BoxError> {
    // One attempt: retrying a failed claim is left to the caller.
    let claimed = transfer_service
        .claim_transfer(
            transfer,
            Some(ClaimTransferConfig {
                max_retries: 1,
                ..ClaimTransferConfig::default()
            }),
        )
        .await?;
    let node_ids: Vec<String> = claimed.iter().map(|node| node.id.to_string()).collect();
    signing_keys.mark_signing_under_node_id(&node_ids).await?;
    tree_store.add_leaves(&claimed).await?;
    Ok(claimed)
}

/// Returns the leaves a reservation holds to the pool, given their ids.
///
/// The store returns a leaf it is told to keep even when that reservation is
/// gone, so this must not run twice for one reservation: its leaves may have been
/// reserved again in between. Callers forget a reservation before releasing it,
/// leaving its leaves reserved if the release does not happen.
pub async fn release_reserved_leaves(
    tree_store: &dyn TreeStore,
    reservation_id: &LeavesReservationId,
    leaf_ids: &[TreeNodeId],
) -> Result<(), TreeServiceError> {
    let leaves: Vec<TreeNode> = tree_store
        .get_leaves()
        .await?
        .reserved_for_payment
        .into_iter()
        .filter(|leaf| leaf_ids.contains(&leaf.id))
        .collect();
    tree_store.cancel_reservation(reservation_id, &leaves).await
}
