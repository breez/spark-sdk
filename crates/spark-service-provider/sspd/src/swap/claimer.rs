use std::sync::Arc;
use std::time::Duration;

use spark::services::TransferStatus;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::in_flight::InFlight;
use crate::leaves::{ClaimDeps, claim_into_pool};
use crate::pool::operator_view;
use crate::swap::SwapStore;
use crate::swap::repository::{SwapDetail, SwapLeaf};
use crate::wakeup::Wakeup;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A new swap wakes the loop. While a swap is left unsettled, it tries again
/// after `CLAIM_RETRY_DELAY`, doubling up to the backup interval.
const CLAIM_BACKUP_INTERVAL: Duration = Duration::from_secs(60);
const CLAIM_RETRY_DELAY: Duration = Duration::from_secs(1);

pub struct SwapClaimDeps {
    pub swap_repo: Arc<dyn SwapStore>,
    pub claim: Arc<ClaimDeps>,
    pub requests: Arc<InFlight>,
    pub wakeup: Wakeup,
}

pub async fn run_swap_claim_loop(deps: SwapClaimDeps, token: CancellationToken) {
    info!("Starting swap claim loop");

    let mut retry: Option<Duration> = None;
    loop {
        tokio::select! {
            () = token.cancelled() => {
                info!("Swap claim loop cancelled");
                return;
            }
            () = deps.wakeup.waited() => retry = None,
            () = tokio::time::sleep(retry.unwrap_or(CLAIM_BACKUP_INTERVAL)) => {}
        }

        let unsettled = claim_pending_swaps(&deps).await.unwrap_or_else(|e| {
            error!("Swap claim check failed: {e}");
            true
        });
        retry = unsettled.then(|| {
            retry.map_or(CLAIM_RETRY_DELAY, |delay| {
                delay.saturating_mul(2).min(CLAIM_BACKUP_INTERVAL)
            })
        });
    }
}

/// Returns whether any swap is left unsettled.
pub async fn claim_pending_swaps(deps: &SwapClaimDeps) -> Result<bool, BoxError> {
    let swaps = deps.swap_repo.get_unclaimed_swaps().await?;
    if swaps.is_empty() {
        return Ok(false);
    }

    let futures = swaps.into_iter().map(|swap| claim_one_swap(deps, swap));
    for result in futures::future::join_all(futures).await {
        if let Err(e) = result {
            error!("failed to claim swap: {e}");
        }
    }
    Ok(!deps.swap_repo.get_unclaimed_swaps().await?.is_empty())
}

async fn claim_one_swap(deps: &SwapClaimDeps, detail: SwapDetail) -> Result<(), BoxError> {
    let swap = &detail.swap;
    // While the swap service serves the request for this swap, it may be
    // creating the counter transfer with the leaves that the claim loop would
    // release here.
    let Some(_request) = deps.requests.start(&swap.id) else {
        return Ok(());
    };
    let Some(transfer) = deps
        .claim
        .transfer_service
        .query_transfer(&swap.user_transfer_id)
        .await?
    else {
        return Ok(());
    };

    // The operators commit both sides of a swap together, so an expired or
    // returned primary transfer means the counter transfer never went through and
    // the reserved leaves are still the SSP's.
    if matches!(
        transfer.status,
        TransferStatus::Expired | TransferStatus::Returned
    ) {
        operator_view::release(
            &deps.claim.operator_pool,
            deps.claim.network,
            &deps.claim.leaf_store,
            &swap.counter_transfer_id,
        )
        .await?;
        deps.swap_repo.delete_swap(&swap.id).await?;
        info!(swap_id = %swap.id, "Swap never went through; its leaves are back in the pool");
        return Ok(());
    }

    if !is_claimable(transfer.status) {
        return Ok(());
    }

    // The operators have made the counter transfer, but the swap service may
    // not have marked its leaves transferred.
    deps.claim
        .leaf_store
        .transferred(&swap.counter_transfer_id)
        .await?;

    info!(swap_id = %swap.id, status = %transfer.status, "Claiming user leaves for swap");
    let claimed = claim_into_pool(&deps.claim, &transfer).await?;

    let inbound: Vec<SwapLeaf> = claimed
        .iter()
        .map(|node| SwapLeaf {
            leaf_id: node.id.to_string(),
            value_sats: i64::try_from(node.value).unwrap_or(i64::MAX),
        })
        .collect();
    deps.swap_repo
        .record_inbound_leaves(&swap.id, &inbound)
        .await?;
    info!(swap_id = %swap.id, count = inbound.len(), "Claimed user leaves into the pool");
    Ok(())
}

/// Includes `Completed`: claiming again hands back the leaves the SSP still owns,
/// finishing a claim that completed at the operators but not here.
fn is_claimable(status: TransferStatus) -> bool {
    matches!(
        status,
        TransferStatus::SenderKeyTweaked
            | TransferStatus::ReceiverKeyTweaked
            | TransferStatus::ReceiverKeyTweakLocked
            | TransferStatus::ReceiverKeyTweakApplied
            | TransferStatus::ReceiverRefundSigned
            | TransferStatus::Completed
    )
}
