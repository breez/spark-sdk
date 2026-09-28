use std::sync::Arc;
use std::time::Duration;

use spark::tree::{TreeNode, TreeNodeStatus, assemble_exit_chains, chain_reaches_root};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::wakeup::Wakeup;
use crate::{
    chain::ChainRepository,
    pool::{
        repository::DepositTree,
        restock::{PoolEvent, RestockService},
    },
    postgresql::PoolRepository,
    wallet::SspWallet,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const REQUIRED_CONFIRMATIONS: u64 = 3;

/// The loop also wakes on every block, which is what confirms a tree.
const BACKUP_INTERVAL: Duration = Duration::from_secs(600);

/// How soon a pass that failed or found a confirmed tree not yet available is tried
/// again.
const OPERATOR_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// The operators refuse a node query by more ids than this.
const MAX_NODE_IDS_PER_QUERY: usize = 1000;

pub async fn run_tree_pooling_loop<R>(
    wallet: Arc<SspWallet<R>>,
    chain_repository: Arc<R>,
    pool_repo: Arc<PoolRepository>,
    restock: Arc<RestockService>,
    chain_advanced: Wakeup,
    token: CancellationToken,
) where
    R: ChainRepository + Send + Sync + 'static,
{
    info!("Starting tree pooling loop");

    loop {
        let retry_in =
            match pool_confirmed_trees(&wallet, &chain_repository, &pool_repo, &restock).await {
                Ok(false) => BACKUP_INTERVAL,
                Ok(true) => OPERATOR_RETRY_INTERVAL,
                Err(e) => {
                    error!("Tree pooling failed: {e}");
                    OPERATOR_RETRY_INTERVAL
                }
            };
        tokio::select! {
            () = token.cancelled() => {
                info!("Tree pooling loop cancelled");
                return;
            }
            () = chain_advanced.waited() => {}
            () = tokio::time::sleep(retry_in) => {}
        }
    }
}

/// Returns whether to try again soon: a tree could not be pooled or is waiting for
/// the operators.
async fn pool_confirmed_trees<R>(
    wallet: &SspWallet<R>,
    chain_repository: &Arc<R>,
    pool_repo: &PoolRepository,
    restock: &RestockService,
) -> Result<bool, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let height = chain_repository
        .get_tip()
        .await?
        .map_or(0, |tip| tip.height);
    let mut waiting = false;
    for deposit in pool_repo.open_deposits().await? {
        for tree in &deposit.trees {
            match pool_tree(wallet, chain_repository.as_ref(), pool_repo, tree, height).await {
                Ok(Pooling::Pooled(values)) => restock.publish(PoolEvent::LeavesAvailable {
                    deposit_address: tree.deposit_address.clone(),
                    denominations: values,
                }),
                Ok(Pooling::Unconfirmed) => {}
                Ok(Pooling::WaitingForOperators) => waiting = true,
                Err(e) => {
                    warn!("could not pool the tree on {}: {e}", tree.outpoint);
                    waiting = true;
                }
            }
        }
    }
    Ok(waiting)
}

enum Pooling {
    /// The values of the leaves that joined the pool.
    Pooled(Vec<u64>),
    Unconfirmed,
    WaitingForOperators,
}

/// Adds a tree's leaves to the leaf store once its funding output has enough
/// confirmations and the operators report the leaves available.
async fn pool_tree<R>(
    wallet: &SspWallet<R>,
    chain_repository: &R,
    pool_repo: &PoolRepository,
    tree: &DepositTree,
    height: u64,
) -> Result<Pooling, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let address = tree
        .deposit_address
        .parse::<bitcoin::Address<_>>()?
        .assume_checked();
    let confirmed = chain_repository
        .get_txos_for_address(&address)
        .await?
        .into_iter()
        .any(|txo| {
            txo.outpoint == tree.outpoint && txo.confirmations(height) >= REQUIRED_CONFIRMATIONS
        });
    if !confirmed {
        return Ok(Pooling::Unconfirmed);
    }
    let nodes = pool_repo.tree_nodes(&tree.outpoint).await?;

    // A leaf no longer reported as the SSP's was spent after an earlier pass stored
    // it without marking the tree pooled.
    let ids: Vec<_> = nodes.leaves.iter().map(|leaf| leaf.id.clone()).collect();
    let identity = wallet.spark.identity_public_key;
    let mut leaves: Vec<TreeNode> = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(MAX_NODE_IDS_PER_QUERY) {
        leaves.extend(
            wallet
                .spark
                .tree_service
                .fetch_nodes(chunk, false)
                .await?
                .into_iter()
                .filter(|node| node.owner_identity_public_key == Some(identity)),
        );
    }
    if leaves
        .iter()
        .any(|leaf| leaf.status != TreeNodeStatus::Available)
    {
        return Ok(Pooling::WaitingForOperators);
    }

    wallet.spark.tree_store.add_leaves(&leaves).await?;

    let by_id = nodes
        .leaves
        .iter()
        .chain(&nodes.branches)
        .map(|node| (node.id.clone(), node.clone()))
        .collect();
    let pooled_ids: Vec<_> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
    let rooted: Vec<_> = assemble_exit_chains(&by_id, &pooled_ids)
        .into_iter()
        .filter(|p| chain_reaches_root(&p.leaf, &p.ancestors))
        .collect();
    if rooted.len() < pooled_ids.len() {
        warn!(
            "{} of {} leaves on {} have no chain to the root, and cannot be exited unilaterally",
            pooled_ids.len().saturating_sub(rooted.len()),
            pooled_ids.len(),
            tree.outpoint,
        );
    }
    wallet.spark.tree_store.store_ancestors(&rooted).await?;

    pool_repo.mark_pooled(&tree.outpoint).await?;
    info!(
        "{} leaves on {} joined the pool",
        leaves.len(),
        tree.outpoint
    );
    Ok(Pooling::Pooled(
        leaves.iter().map(|leaf| leaf.value).collect(),
    ))
}
