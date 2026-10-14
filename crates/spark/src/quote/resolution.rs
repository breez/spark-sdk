//! Resolution of rate (bps) edges against the gross a payer actually sent.
//! Sats edges are flat and come off the top; bps edges divide the remainder.

use super::QuoteError;
use crate::operator::rpc::spark::ManifestEdge;
use crate::operator::rpc::spark::manifest_amount::Amount;

const BPS_DENOMINATOR: u64 = 10_000;

/// One edge's amount once the gross is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedEdge {
    /// The floored share the operators expect.
    pub resolved_sats: u64,
    /// The inclusive upper bound the operators accept. One above `resolved_sats`
    /// on a bps edge, so the SSP has room to place the rounding dust.
    pub allowed_sats: u64,
}

/// Resolves each edge against `gross`, in edge order, as the operators do when
/// binding a manifest.
pub fn resolve_manifest_edges(
    edges: &[ManifestEdge],
    gross: u64,
) -> Result<Vec<ResolvedEdge>, QuoteError> {
    Ok(resolve(edges, gross)?
        .into_iter()
        .map(|(edge, _)| edge)
        .collect())
}

/// Where the SSP places each edge's sats: the resolved shares, plus the
/// rounding dust handed out one sat at a time to the bps edges with the largest
/// remainder, ties going to the earlier edge. The operators only bound this
/// placement, so it is what a payee actually receives on today's SSP rather
/// than a protocol guarantee.
pub fn allocate_manifest_edges(edges: &[ManifestEdge], gross: u64) -> Result<Vec<u64>, QuoteError> {
    let resolved = resolve(edges, gross)?;
    let mut allocated: Vec<u64> = resolved
        .iter()
        .map(|(edge, _)| edge.resolved_sats)
        .collect();
    let placed = allocated.iter().copied().fold(0u64, u64::saturating_add);
    let dust = gross.saturating_sub(placed);

    let mut by_remainder: Vec<(usize, u64)> = resolved
        .iter()
        .enumerate()
        .filter_map(|(i, (_, remainder))| remainder.map(|remainder| (i, remainder)))
        .collect();
    // Stable, so equal remainders keep manifest order.
    by_remainder.sort_by_key(|(_, remainder)| std::cmp::Reverse(*remainder));
    for (i, _) in by_remainder
        .into_iter()
        .take(usize::try_from(dust).unwrap_or(usize::MAX))
    {
        if let Some(sats) = allocated.get_mut(i) {
            *sats = sats.saturating_add(1);
        }
    }
    Ok(allocated)
}

// Each edge's resolution, plus the remainder of its bps division (None for a
// flat edge).
fn resolve(
    edges: &[ManifestEdge],
    gross: u64,
) -> Result<Vec<(ResolvedEdge, Option<u64>)>, QuoteError> {
    let mut flat_sats = 0u64;
    let mut bps_total = 0u64;
    let mut any_bps = false;
    for (i, edge) in edges.iter().enumerate() {
        match amount_of(edge) {
            Some(Amount::Sats(sats)) => {
                flat_sats = flat_sats.checked_add(*sats).ok_or_else(|| {
                    QuoteError::invalid(format!("edges[{i}]: sats edges overflow"))
                })?;
            }
            Some(Amount::Bps(bps)) => {
                any_bps = true;
                bps_total = bps_total.saturating_add(u64::from(*bps));
            }
            None => return Err(QuoteError::invalid(format!("edges[{i}] has no amount"))),
        }
    }

    let mut remainder = gross;
    if any_bps {
        // Rates that do not partition the remainder would admit a manifest
        // declaring only part of the gross.
        if bps_total != BPS_DENOMINATOR {
            return Err(QuoteError::invalid(format!(
                "manifest bps edges must sum to {BPS_DENOMINATOR}: edges sum to {bps_total}"
            )));
        }
        let leftover = "manifest sats edges leave nothing for the bps edges to divide";
        if gross == 0 {
            return Err(QuoteError::invalid(format!(
                "{leftover}: no executed value to divide"
            )));
        }
        if flat_sats >= gross {
            return Err(QuoteError::invalid(format!(
                "{leftover}: {flat_sats} sats of {gross}"
            )));
        }
        remainder = gross - flat_sats;
    }

    Ok(edges
        .iter()
        .filter_map(amount_of)
        .map(|amount| match amount {
            Amount::Sats(sats) => (
                ResolvedEdge {
                    resolved_sats: *sats,
                    allowed_sats: *sats,
                },
                None,
            ),
            Amount::Bps(bps) => {
                // u128 because remainder * 10000 overflows u64 near the total
                // bitcoin supply.
                let product = u128::from(remainder) * u128::from(*bps);
                let denominator = u128::from(BPS_DENOMINATOR);
                let resolved = u64::try_from(product / denominator).unwrap_or(u64::MAX);
                let fraction = u64::try_from(product % denominator).unwrap_or_default();
                (
                    ResolvedEdge {
                        resolved_sats: resolved,
                        allowed_sats: resolved.saturating_add(1),
                    },
                    Some(fraction),
                )
            }
        })
        .collect())
}

fn amount_of(edge: &ManifestEdge) -> Option<&Amount> {
    edge.amount
        .as_ref()
        .and_then(|amount| amount.amount.as_ref())
}
