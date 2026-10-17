//! Renewal of the pool leaves whose refund needs renewal before the SSP can
//! transfer them again. A leaf that the SSP claims can have such a refund. The
//! SSP renews leaves here, and not while it claims them, because renewing a
//! leaf takes a round trip to the operators.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use spark::operator::OperatorPool;
use spark::services::{RenewalCandidate, TimelockManager};
use spark::signer::LeafSigningKey;
use spark::tree::{LeafPedigree, TreeNode, TreeNodeId};
use tracing::{info, warn};

use crate::pool::operator_view::query_leaves;
use crate::postgresql::{PostgresLeafStore, RenewalCursor};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct RenewalDeps {
    pub leaf_store: Arc<PostgresLeafStore>,
    pub timelocks: Arc<TimelockManager>,
    pub operator_pool: Arc<OperatorPool>,
    pub network: spark::Network,
}

/// The SSP renews the leaves of a batch at the same time, with a call to the
/// coordinator for each, so the batch size is the most renewals it requests at
/// once.
const CONCURRENT_RENEWALS: usize = 64;

/// After this many failed attempts at renewing a leaf, the SSP warns about the
/// leaf. This is not a limit: the SSP keeps trying.
const STUCK_AFTER_ATTEMPTS: u32 = 5;

#[derive(Default)]
pub struct Attempts(HashMap<TreeNodeId, u32>);

impl Attempts {
    fn failed(&mut self, leaf: &TreeNodeId) {
        let attempts = self.0.entry(leaf.clone()).or_default();
        *attempts = attempts.saturating_add(1);
        if *attempts >= STUCK_AFTER_ATTEMPTS {
            warn!(
                leaf_id = %leaf,
                attempts = *attempts,
                "renewing a pool leaf has failed repeatedly, and it stays unavailable",
            );
        }
    }

    fn renewed(&mut self, leaf: &TreeNodeId) {
        self.0.remove(leaf);
    }
}

pub async fn renew_pass(deps: &RenewalDeps, attempts: &mut Attempts) -> Result<usize, BoxError> {
    let mut cursor = RenewalCursor::default();
    let mut renewed: usize = 0;
    loop {
        let (leaves, next) = deps
            .leaf_store
            .leaves_needing_renewal(cursor, CONCURRENT_RENEWALS)
            .await?;
        renewed = renewed.saturating_add(renew(deps, attempts, leaves).await?);
        match next {
            Some(next) => cursor = next,
            None => return Ok(renewed),
        }
    }
}

/// When the SSP requests a renewal that the operators made in the last day,
/// the operators return the leaf they made then. So when the SSP never
/// received the result of a renewal, it gets the renewed leaf by requesting
/// the renewal again with the refund it still has.
async fn renew(
    deps: &RenewalDeps,
    attempts: &mut Attempts,
    leaves: Vec<LeafPedigree>,
) -> Result<usize, BoxError> {
    if leaves.is_empty() {
        return Ok(0);
    }
    let before: HashMap<TreeNodeId, TreeNode> = leaves
        .iter()
        .map(|pedigree| (pedigree.leaf.id.clone(), pedigree.leaf.clone()))
        .collect();
    // Only a claimed leaf needs renewal, and the signer derives a claimed
    // leaf's signing key from the leaf's node id.
    let candidates = leaves
        .into_iter()
        .map(|pedigree| RenewalCandidate {
            signing_key: LeafSigningKey {
                derived_from: pedigree.leaf.id.clone(),
            },
            pedigree,
        })
        .collect();

    let queried_at = Utc::now();
    let checked = match deps.timelocks.check_renew_nodes(candidates).await {
        Ok(checked) => checked,
        Err(e) => {
            warn!("the operators did not renew a batch of pool leaves: {e}");
            let failed: Vec<TreeNodeId> = before.keys().cloned().collect();
            for leaf in &failed {
                attempts.failed(leaf);
            }
            query_leaves(&deps.operator_pool, deps.network, &deps.leaf_store, &failed).await;
            return Ok(0);
        }
    };
    // The timelock manager also returns the leaves it did not renew, as they
    // were. Such a leaf still has its old refund.
    let mut replaced = Vec::new();
    let mut renewed = Vec::new();
    let mut failed = Vec::new();
    for pedigree in checked {
        let old = before.get(&pedigree.leaf.id);
        if old.is_some_and(|old| refund_txid(old) != refund_txid(&pedigree.leaf)) {
            attempts.renewed(&pedigree.leaf.id);
            replaced.extend(old.cloned());
            renewed.push(pedigree);
        } else {
            attempts.failed(&pedigree.leaf.id);
            failed.push(pedigree.leaf.id);
        }
    }
    query_leaves(&deps.operator_pool, deps.network, &deps.leaf_store, &failed).await;
    if renewed.is_empty() {
        return Ok(0);
    }
    deps.leaf_store
        .renewed(&replaced, &renewed, queried_at)
        .await?;
    info!("renewed {} pool leaves", renewed.len());
    Ok(renewed.len())
}

fn refund_txid(leaf: &TreeNode) -> Option<bitcoin::Txid> {
    leaf.refund_tx
        .as_ref()
        .map(bitcoin::Transaction::compute_txid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leaf_that_was_renewed_starts_over() {
        let leaf: TreeNodeId = "leaf".parse().unwrap();
        let mut attempts = Attempts::default();
        attempts.failed(&leaf);
        attempts.failed(&leaf);
        assert_eq!(attempts.0.get(&leaf), Some(&2));

        attempts.renewed(&leaf);

        assert_eq!(attempts.0.get(&leaf), None);
    }
}
