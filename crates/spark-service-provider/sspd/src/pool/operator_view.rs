//! The store's record of what the operators report for the SSP's leaves, and
//! the code that keeps it up to date.
//!
//! The SSP decides for itself which leaves are its own, and uses a leaf only
//! while the operators agree. The operators can stop agreeing after the SSP has
//! added a leaf to the pool. For example, they report a leaf as `PARENT_EXITED`
//! once its parent's node transaction confirms. The SSP writes what the
//! operators report to this record only, and never changes what it has stored
//! of the leaf itself.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use spark::operator::OperatorPool;
use spark::services::TransferId;
use spark::tree::TreeNodeId;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use crate::operator_rpc::{self, MAX_NODE_IDS_PER_QUERY, NodeQuery};
use crate::postgresql::{Dispute, LeafStoreError, PostgresLeafStore};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const PASS_PAUSE: Duration = Duration::from_mins(5);

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_mins(1);

pub struct OperatorViewDeps {
    pub operator_pool: Arc<OperatorPool>,
    pub network: spark::Network,
    pub leaf_store: Arc<PostgresLeafStore>,
}

pub async fn run_operator_view_loop(deps: OperatorViewDeps, token: CancellationToken) {
    let mut lasting = Lasting::default();
    loop {
        let mut after: Option<TreeNodeId> = None;
        let mut backoff = INITIAL_BACKOFF;
        let mut disputes = Vec::new();
        while !token.is_cancelled() {
            match query_next(&deps, after.as_ref()).await {
                Ok(Some((last, found))) => {
                    after = Some(last);
                    disputes.extend(found);
                    backoff = INITIAL_BACKOFF;
                }
                Ok(None) => break,
                Err(e) => {
                    error!("could not query the operators for the pool's leaves: {e}");
                    tokio::select! {
                        () = token.cancelled() => return,
                        () = tokio::time::sleep(backoff) => {}
                    }
                    backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                }
            }
        }
        for dispute in lasting.seen_again(disputes) {
            warn!("{dispute}");
        }
        tokio::select! {
            () = token.cancelled() => return,
            () = tokio::time::sleep(PASS_PAUSE) => {}
        }
    }
}

async fn query_next(
    deps: &OperatorViewDeps,
    after: Option<&TreeNodeId>,
) -> Result<Option<(TreeNodeId, Vec<Dispute>)>, BoxError> {
    let queried = deps
        .leaf_store
        .node_ids(after, MAX_NODE_IDS_PER_QUERY)
        .await?;
    let Some(last) = queried.last().cloned() else {
        return Ok(None);
    };
    let disputes = query_operators(
        &deps.operator_pool,
        deps.network,
        &deps.leaf_store,
        &queried,
    )
    .await?;
    Ok(Some((last, disputes)))
}

async fn query_operators(
    operator_pool: &OperatorPool,
    network: spark::Network,
    leaf_store: &PostgresLeafStore,
    queried: &[TreeNodeId],
) -> Result<Vec<Dispute>, BoxError> {
    // The function takes the time before it queries the operators. The store
    // keeps a report that it recorded for a leaf after this time, because what
    // the operators return now may be older than that report.
    let queried_at = Utc::now();
    let client = &operator_pool.get_coordinator().client;
    let reported = operator_rpc::nodes(client, network, queried, false, NodeQuery::Public).await?;
    leaf_store
        .operators_reported(queried, &reported, queried_at)
        .await?;
    let disputes = leaf_store.disputes_among(queried).await?;
    Ok(disputes.into_iter().map(|d| d.dispute).collect())
}

/// The function queries the operators for the released leaves, because one of
/// them may be the reason the operators did not complete the transfer. The
/// store's record of what the operators report for the leaves may also date
/// from while the transfer was under way.
pub async fn release(
    operator_pool: &OperatorPool,
    network: spark::Network,
    leaf_store: &PostgresLeafStore,
    transfer_id: &TransferId,
) -> Result<(), LeafStoreError> {
    let released = leaf_store.release(transfer_id).await?;
    query_leaves(operator_pool, network, leaf_store, &released).await;
    Ok(())
}

/// The SSP stops using a leaf in dispute once this function has recorded the
/// operators' report, without waiting for the loop to reach the leaf.
pub async fn query_leaves(
    operator_pool: &OperatorPool,
    network: spark::Network,
    leaf_store: &PostgresLeafStore,
    leaves: &[TreeNodeId],
) {
    if leaves.is_empty() {
        return;
    }
    match query_operators(operator_pool, network, leaf_store, leaves).await {
        Ok(disputes) => {
            for dispute in disputes {
                warn!("{dispute}");
            }
        }
        Err(e) => {
            warn!("could not query the operators for leaves right away: {e}");
        }
    }
}

/// The loop logs a dispute once, when it sees the dispute in a second pass in a
/// row. A dispute that is gone by the next pass is the result of a change to
/// the leaf while the SSP queried the operators for it. A dispute that lasts is
/// a fault.
#[derive(Default)]
struct Lasting {
    /// Each dispute of the last pass, and whether [`Self::seen_again`] has
    /// returned it.
    seen: HashMap<Dispute, bool>,
}

impl Lasting {
    fn seen_again(&mut self, disputes: Vec<Dispute>) -> Vec<Dispute> {
        let mut again = Vec::new();
        let mut seen = HashMap::with_capacity(disputes.len());
        for dispute in disputes {
            let returned = match self.seen.get(&dispute) {
                None => false,
                Some(true) => true,
                Some(false) => {
                    again.push(dispute.clone());
                    true
                }
            };
            seen.insert(dispute, returned);
        }
        self.seen = seen;
        again
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgresql::Reported;

    fn dispute(node_id: &str, reported: Reported) -> Dispute {
        Dispute {
            node_id: node_id.parse().unwrap(),
            reported,
        }
    }

    #[test]
    fn a_dispute_is_returned_once_when_it_is_seen_a_second_time() {
        let unreported = dispute("unreported", Reported::Unreported);
        let mut lasting = Lasting::default();

        assert!(lasting.seen_again(vec![unreported.clone()]).is_empty());
        assert_eq!(
            lasting.seen_again(vec![unreported.clone()]),
            std::slice::from_ref(&unreported)
        );
        assert!(lasting.seen_again(vec![unreported]).is_empty());
    }

    #[test]
    fn a_dispute_gone_after_the_next_report_is_not_returned() {
        let locked = dispute("leaf", Reported::Status("TransferLocked".to_string()));
        let mut lasting = Lasting::default();

        assert!(lasting.seen_again(vec![locked.clone()]).is_empty());
        assert!(lasting.seen_again(Vec::new()).is_empty());
        assert!(lasting.seen_again(vec![locked]).is_empty());
    }

    #[test]
    fn a_leaf_the_operators_report_differently_is_another_dispute() {
        let leaf = |reported| dispute("leaf", reported);
        let mut lasting = Lasting::default();

        assert!(
            lasting
                .seen_again(vec![leaf(Reported::Unreported)])
                .is_empty()
        );
        let exited = leaf(Reported::Status("ParentExited".to_string()));
        assert!(lasting.seen_again(vec![exited.clone()]).is_empty());
        assert_eq!(lasting.seen_again(vec![exited.clone()]), [exited]);
    }
}
