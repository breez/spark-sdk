use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::operator_rpc::{self, NodeQuery};
use chrono::Utc;
use spark::tree::{
    LeafPedigree, TreeNode, TreeNodeId, TreeNodeStatus, assemble_exit_chains, chain_reaches_root,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::wakeup::Wakeup;
use crate::{
    chain::{ChainRepository, ChainTip},
    pool::{
        repository::{DepositTree, DepositTx},
        restock::{PoolEvent, RestockService},
    },
    postgresql::PoolRepository,
    wallet::SspWallet,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The loop also wakes on every block, which is what confirms a tree.
const BACKUP_INTERVAL: Duration = Duration::from_secs(600);

/// How soon a pass that failed or found a confirmed tree not yet available is tried
/// again.
const OPERATOR_RETRY_INTERVAL: Duration = Duration::from_secs(10);

pub struct TreePoolingDeps<R: ChainRepository> {
    pub wallet: Arc<SspWallet<R>>,
    pub chain_repository: Arc<R>,
    pub chain_tip: ChainTip,
    pub pool_repo: Arc<PoolRepository>,
    pub restock: Arc<RestockService>,
    pub confirmations: u64,
}

pub async fn run_tree_pooling_loop<R>(
    deps: TreePoolingDeps<R>,
    chain_advanced: Wakeup,
    token: CancellationToken,
) where
    R: ChainRepository + Send + Sync + 'static,
{
    let TreePoolingDeps {
        wallet,
        chain_repository,
        chain_tip,
        pool_repo,
        restock,
        confirmations,
    } = deps;
    info!("Starting tree pooling loop");

    loop {
        let retry_in = match pool_confirmed_trees(
            &wallet,
            &chain_repository,
            &chain_tip,
            &pool_repo,
            &restock,
            confirmations,
        )
        .await
        {
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
    chain_tip: &ChainTip,
    pool_repo: &PoolRepository,
    restock: &RestockService,
    confirmations: u64,
) -> Result<bool, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let height = chain_tip.record().map_or(0, |tip| tip.height);
    let deposits = pool_repo.open_deposits().await?;
    let txos = tree_txos(chain_repository.as_ref(), &deposits).await?;
    let mut waiting = false;
    for deposit in deposits {
        for tree in &deposit.trees {
            match pool_tree(wallet, &txos, pool_repo, tree, height, confirmations).await {
                Ok(Pooling::Pooled(values)) => restock.publish(PoolEvent::LeavesAvailable {
                    deposit_address: tree.deposit_address.clone(),
                    denominations: values,
                }),
                Ok(Pooling::Unconfirmed | Pooling::AlreadyPooled) => {}
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

async fn tree_txos<R: ChainRepository>(
    chain_repository: &R,
    deposits: &[DepositTx],
) -> Result<Vec<crate::chain::Txo>, BoxError> {
    let mut addresses = Vec::new();
    for tree in deposits.iter().flat_map(|deposit| &deposit.trees) {
        match tree.deposit_address.parse::<bitcoin::Address<_>>() {
            Ok(address) => addresses.push(address.assume_checked()),
            Err(e) => warn!("the tree on {} has an invalid address: {e}", tree.outpoint),
        }
    }
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    Ok(chain_repository.get_txos_for_addresses(&addresses).await?)
}

enum Pooling {
    /// The values of the leaves that joined the pool.
    Pooled(Vec<u64>),
    Unconfirmed,
    WaitingForOperators,
    AlreadyPooled,
}

async fn pool_tree<R>(
    wallet: &SspWallet<R>,
    txos: &[crate::chain::Txo],
    pool_repo: &PoolRepository,
    tree: &DepositTree,
    height: u64,
    confirmations: u64,
) -> Result<Pooling, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let confirmed = txos
        .iter()
        .any(|txo| txo.outpoint == tree.outpoint && txo.confirmations(height) >= confirmations);
    if !confirmed {
        return Ok(Pooling::Unconfirmed);
    }
    let nodes = pool_repo
        .tree_nodes(&tree.outpoint)
        .await?
        .ok_or("the store has no nodes for the tree")?;

    let queried_at = Utc::now();
    let ids: Vec<_> = nodes.leaves.iter().map(|leaf| leaf.id.clone()).collect();
    let identity = wallet.spark.identity_public_key;
    let client = &wallet.spark.operator_pool.get_coordinator().client;
    let leaves: Vec<TreeNode> =
        operator_rpc::nodes(client, wallet.spark.network, &ids, false, NodeQuery::Public)
            .await?
            .into_iter()
            .filter(|node| node.owner_identity_public_key == Some(identity))
            .collect();
    if leaves
        .iter()
        .any(|leaf| leaf.status != TreeNodeStatus::Available)
    {
        return Ok(Pooling::WaitingForOperators);
    }

    let by_id = nodes
        .leaves
        .iter()
        .chain(&nodes.branches)
        .map(|node| (node.id.clone(), node.clone()))
        .collect();
    let pooled_ids: Vec<_> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
    let mut ancestors: HashMap<TreeNodeId, Vec<TreeNode>> =
        assemble_exit_chains(&by_id, &pooled_ids)
            .into_iter()
            .filter(|p| chain_reaches_root(&p.leaf, &p.ancestors))
            .map(|p| (p.leaf.id, p.ancestors))
            .collect();
    // The SSP needs every node above a leaf up to the root to exit the leaf
    // unilaterally. The function adds none of the tree's leaves unless each one
    // has those nodes.
    if ancestors.len() < pooled_ids.len() {
        return Err(format!(
            "{} of {} leaves on {} have no chain to the root",
            pooled_ids.len().saturating_sub(ancestors.len()),
            pooled_ids.len(),
            tree.outpoint,
        )
        .into());
    }
    let pedigrees: Vec<LeafPedigree> = leaves
        .iter()
        .map(|leaf| LeafPedigree {
            ancestors: ancestors.remove(&leaf.id).unwrap_or_default(),
            leaf: leaf.clone(),
        })
        .collect();
    let pooled = pool_repo
        .pool_tree(
            &tree.outpoint,
            &wallet.spark.leaf_store,
            &pedigrees,
            queried_at,
        )
        .await?;
    if !pooled {
        return Ok(Pooling::AlreadyPooled);
    }
    for leaf in &leaves {
        let refund_txid = leaf
            .refund_tx
            .as_ref()
            .map(bitcoin::Transaction::compute_txid);
        info!(leaf_id = %leaf.id, ?refund_txid, tree = %tree.outpoint, "a leaf of the SSP's own tree joined the pool");
    }
    info!(
        "{} leaves on {} joined the pool",
        leaves.len(),
        tree.outpoint
    );
    Ok(Pooling::Pooled(
        leaves.iter().map(|leaf| leaf.value).collect(),
    ))
}
