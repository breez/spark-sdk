use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use spark::services::{TransferId, TransferService, TransferStatus};
use spark::tree::TreeNodeId;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::leaves::{IncomingLeafStore, LeafSigningKeys, claim_into_pool, release_reserved_leaves};
use crate::swap::SwapStore;
use crate::swap::repository::{SwapDetail, SwapLeaf};
use crate::wakeup::Wakeup;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A new swap wakes the loop, so the timer only retries swaps that could not be
/// settled yet.
const CLAIM_BACKUP_INTERVAL: Duration = Duration::from_secs(60);

pub struct SwapClaimDeps {
    pub swap_repo: Arc<dyn SwapStore>,
    pub tree_store: Arc<dyn spark::tree::TreeStore>,
    pub transfer_service: Arc<TransferService>,
    pub incoming: Arc<dyn IncomingLeafStore>,
    pub admission: Wakeup,
    pub leaf_signing_keys: Arc<dyn LeafSigningKeys>,
    pub wakeup: Wakeup,
}

pub async fn run_swap_claim_loop(deps: SwapClaimDeps, token: CancellationToken) {
    info!("Starting swap claim loop");

    loop {
        tokio::select! {
            () = token.cancelled() => {
                info!("Swap claim loop cancelled");
                return;
            }
            () = deps.wakeup.waited() => {}
            () = tokio::time::sleep(CLAIM_BACKUP_INTERVAL) => {}
        }

        if let Err(e) = claim_pending_swaps(&deps).await {
            error!("Swap claim check failed: {e}");
        }
    }
}

pub async fn claim_pending_swaps(deps: &SwapClaimDeps) -> Result<(), BoxError> {
    let swaps = deps.swap_repo.get_unclaimed_swaps().await?;
    if swaps.is_empty() {
        return Ok(());
    }

    let futures = swaps.into_iter().map(|swap| claim_one_swap(deps, swap));
    for result in futures::future::join_all(futures).await {
        if let Err(e) = result {
            error!("failed to claim swap: {e}");
        }
    }
    Ok(())
}

async fn claim_one_swap(deps: &SwapClaimDeps, detail: SwapDetail) -> Result<(), BoxError> {
    let swap = &detail.swap;
    let transfer_id = TransferId::from_str(&swap.user_transfer_id)
        .map_err(|e| format!("invalid user transfer id {}: {e}", swap.user_transfer_id))?;

    let Some(transfer) = deps.transfer_service.query_transfer(&transfer_id).await? else {
        return Ok(());
    };

    // The operators commit both sides of a swap together, so an expired or
    // returned primary transfer means the counter transfer never went through and
    // the reserved leaves are still the SSP's.
    if matches!(
        transfer.status,
        TransferStatus::Expired | TransferStatus::Returned
    ) {
        let outbound = detail
            .outbound
            .iter()
            .map(|leaf| TreeNodeId::from_str(&leaf.leaf_id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("invalid outbound leaf id: {e}"))?;
        release_reserved_leaves(deps.tree_store.as_ref(), &swap.reservation_id, &outbound).await?;
        deps.swap_repo.delete_swap(&swap.id).await?;
        info!(swap_id = %swap.id, "Swap never went through; its leaves are back in the pool");
        return Ok(());
    }

    if !is_claimable(transfer.status) {
        return Ok(());
    }

    // The counter transfer went through, but the swap request may not have
    // finalized its reservation.
    deps.tree_store
        .finalize_reservation(&swap.reservation_id, None)
        .await?;

    info!(swap_id = %swap.id, status = %transfer.status, "Claiming user leaves for swap");
    let claimed = claim_into_pool(
        &deps.transfer_service,
        deps.leaf_signing_keys.as_ref(),
        deps.incoming.as_ref(),
        &deps.admission,
        &transfer,
    )
    .await?;

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
