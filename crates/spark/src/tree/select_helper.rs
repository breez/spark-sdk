use tracing::{error, trace};

use crate::tree::{
    LeavesReservation, TargetAmounts, TargetLeaves, TreeNode, TreeService, TreeServiceError,
};

pub fn select_leaves_by_target_amounts(
    leaves: &[TreeNode],
    target_amounts: Option<&TargetAmounts>,
) -> Result<TargetLeaves, TreeServiceError> {
    let mut remaining_leaves = leaves.to_vec();

    // If no target amounts are specified, return all remaining leaves
    let Some(target_amounts) = target_amounts else {
        trace!("No target amounts specified, returning all remaining leaves");
        return Ok(TargetLeaves::new(remaining_leaves, None));
    };

    match target_amounts {
        TargetAmounts::AmountAndFee {
            amount_sats,
            fee_sats,
        } => {
            // Select leaves that match the target amount_sats
            let amount_leaves = select_leaves_by_exact_amount(&remaining_leaves, *amount_sats)?
                .ok_or(TreeServiceError::UnselectableAmount)?;

            let fee_leaves = match fee_sats {
                Some(fee_sats) => {
                    // Remove the amount_leaves from remaining_leaves to avoid double spending
                    remaining_leaves.retain(|leaf| {
                        !amount_leaves
                            .iter()
                            .any(|amount_leaf| amount_leaf.id == leaf.id)
                    });
                    // Select leaves that match the fee_sats from the remaining leaves
                    Some(
                        select_leaves_by_exact_amount(&remaining_leaves, *fee_sats)?
                            .ok_or(TreeServiceError::UnselectableAmount)?,
                    )
                }
                None => None,
            };

            Ok(TargetLeaves::new(amount_leaves, fee_leaves))
        }
        TargetAmounts::ExactDenominations { denominations } => {
            // Select leaves that match the target denominations
            let denominations_leaves =
                select_leaves_by_exact_denominations(&remaining_leaves, denominations)?;
            Ok(TargetLeaves::new(denominations_leaves, None))
        }
    }
}

/// Selects leaves from the tree that sum up to exactly the target amount.
/// If such a combination of leaves does not exist, it returns `None`.
pub fn select_leaves_by_exact_amount(
    leaves: &[TreeNode],
    target_amount_sat: u64,
) -> Result<Option<Vec<TreeNode>>, TreeServiceError> {
    if target_amount_sat == 0 {
        return Err(TreeServiceError::InvalidAmount);
    }

    if leaves.iter().map(|leaf| leaf.value).sum::<u64>() < target_amount_sat {
        return Err(TreeServiceError::InsufficientFunds);
    }

    // Try to find a single leaf that matches the exact amount
    if let Some(leaf) = find_exact_single_match(leaves, target_amount_sat) {
        return Ok(Some(vec![leaf]));
    }

    // Try to find a set of leaves that sum exactly to the target amount
    if let Some(selected_leaves) = find_exact_multiple_match(leaves, target_amount_sat) {
        return Ok(Some(selected_leaves));
    }

    Ok(None)
}

pub fn select_leaves_by_exact_denominations(
    leaves: &[TreeNode],
    denominations: &[u64],
) -> Result<Vec<TreeNode>, TreeServiceError> {
    let mut remaining_leaves = leaves.to_vec();
    let mut selected_leaves = Vec::new();

    for denomination in denominations {
        let leaf = find_exact_single_match(&remaining_leaves, *denomination)
            .ok_or(TreeServiceError::UnselectableAmount)?;
        selected_leaves.push(leaf.clone());
        remaining_leaves.retain(|remaining_leaf| remaining_leaf.id != leaf.id);
    }

    Ok(selected_leaves)
}

/// Selects leaves from the tree that sum up to at least the target amount.
pub(crate) fn select_leaves_by_minimum_amount(
    leaves: &[TreeNode],
    target_amount_sat: u64,
) -> Result<Option<Vec<TreeNode>>, TreeServiceError> {
    if target_amount_sat == 0 {
        return Err(TreeServiceError::InvalidAmount);
    }
    if leaves.iter().map(|leaf| leaf.value).sum::<u64>() < target_amount_sat {
        return Err(TreeServiceError::InsufficientFunds);
    }

    let mut result = Vec::new();
    let mut sum = 0;
    for leaf in leaves {
        sum += leaf.value;
        result.push(leaf.clone());
        if sum >= target_amount_sat {
            break;
        }
    }

    if sum < target_amount_sat {
        return Ok(None);
    }

    Ok(Some(result))
}

pub(crate) fn find_exact_single_match(
    leaves: &[TreeNode],
    target_amount_sat: u64,
) -> Option<TreeNode> {
    leaves
        .iter()
        .find(|leaf| leaf.value == target_amount_sat)
        .cloned()
}

/// Helper to check if a value is a power of two
fn is_power_of_two(value: u64) -> bool {
    value > 0 && (value & (value - 1)) == 0
}

/// Greedy algorithm to find exact match.
/// Sorts leaves by value in descending order and takes the largest leaf that fits
/// the remaining amount until the target is reached or no valid leaf can be found.
fn greedy_exact_match(leaves: &[TreeNode], target_amount_sat: u64) -> Option<Vec<TreeNode>> {
    let mut sorted_leaves = leaves.to_vec();
    sorted_leaves.sort_by(|a, b| b.value.cmp(&a.value));

    let mut result = Vec::new();
    let mut remaining = target_amount_sat;

    for leaf in &sorted_leaves {
        if leaf.value > remaining {
            continue;
        }
        remaining -= leaf.value;
        result.push(leaf.clone());
        if remaining == 0 {
            return Some(result);
        }
    }

    None // Couldn't reach exact target
}

pub(crate) fn find_exact_multiple_match(
    leaves: &[TreeNode],
    target_amount_sat: u64,
) -> Option<Vec<TreeNode>> {
    if target_amount_sat == 0 {
        return Some(Vec::new());
    }
    if leaves.is_empty() {
        return None;
    }

    // Pass 1: Try greedy on all leaves
    if let Some(result) = greedy_exact_match(leaves, target_amount_sat) {
        return Some(result);
    }

    // Pass 2: Try with only power-of-two leaves (if there were non-power-of-two leaves)
    let power_of_two_leaves: Vec<_> = leaves
        .iter()
        .filter(|l| is_power_of_two(l.value))
        .cloned()
        .collect();

    // If all leaves were already power-of-two, no point in retrying
    if power_of_two_leaves.len() == leaves.len() {
        return None;
    }

    greedy_exact_match(&power_of_two_leaves, target_amount_sat)
}

pub async fn with_reserved_leaves<F, R, E>(
    tree_service: &dyn TreeService,
    f: F,
    leaves: &LeavesReservation,
) -> Result<R, E>
where
    F: Future<Output = Result<R, E>>,
{
    match f.await {
        Ok(r) => {
            if let Err(e) = tree_service
                .finalize_reservation(leaves.id.clone(), None)
                .await
            {
                error!("Failed to finalize reservation: {e:?}");
            }
            Ok(r)
        }
        Err(e) => {
            if let Err(e) = tree_service.cancel_reservation(leaves.id.clone()).await {
                error!("Failed to cancel reservation: {e:?}");
            }
            Err(e)
        }
    }
}
