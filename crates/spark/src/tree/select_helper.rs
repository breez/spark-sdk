use tracing::{error, trace};

use crate::tree::{
    LeavesReservation, TargetAmounts, TargetLeaves, TreeNode, TreeService, TreeServiceError,
};

pub fn select_leaves_by_amounts(
    leaves: &[TreeNode],
    target_amounts: Option<&TargetAmounts>,
) -> Result<TargetLeaves, TreeServiceError> {
    let mut remaining_leaves = leaves.to_vec();

    // If no target amounts are specified, return all remaining leaves
    let Some(target_amounts) = target_amounts else {
        trace!("No target amounts specified, returning all remaining leaves");
        return Ok(TargetLeaves::new(remaining_leaves, None));
    };

    // Select leaves that match the target amount_sats
    let amount_leaves = select_leaves_by_amount(&remaining_leaves, target_amounts.amount_sats)?
        .ok_or(TreeServiceError::UnselectableAmount)?;

    let fee_leaves = match target_amounts.fee_sats {
        Some(fee_sats) => {
            // Remove the amount_leaves from remaining_leaves to avoid double spending
            remaining_leaves.retain(|leaf| {
                !amount_leaves
                    .iter()
                    .any(|amount_leaf| amount_leaf.id == leaf.id)
            });
            // Select leaves that match the fee_sats from the remaining leaves
            Some(
                select_leaves_by_amount(&remaining_leaves, fee_sats)?
                    .ok_or(TreeServiceError::UnselectableAmount)?,
            )
        }
        None => None,
    };

    Ok(TargetLeaves::new(amount_leaves, fee_leaves))
}

/// Selects leaves from the tree that sum up to exactly the target amount.
/// If such a combination of leaves does not exist, it returns `None`.
pub fn select_leaves_by_amount(
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

pub(crate) fn find_exact_multiple_match(
    leaves: &[TreeNode],
    target_amount_sat: u64,
) -> Option<Vec<TreeNode>> {
    use std::collections::HashMap;

    // Early return if target is 0 or if there are no leaves
    if target_amount_sat == 0 {
        return Some(Vec::new());
    }
    if leaves.is_empty() {
        return None;
    }

    // Sort leaves by value in descending order, as we want to use larger leaves first.
    // This avoids potentially consuming smaller leaves that could be used later for
    // smaller targets, like paying fees.
    let mut sorted_leaves = leaves.to_vec();
    sorted_leaves.sort_by(|a, b| b.value.cmp(&a.value));

    // Use dynamic programming with HashMap for space efficiency
    // dp[amount] = (leaf_idx, prev_amount) represents that we can achieve 'amount'
    // by using leaf at leaf_idx and then achieve prev_amount
    let mut dp: HashMap<u64, (usize, u64)> = HashMap::new();
    dp.insert(0, (usize::MAX, 0)); // Special marker for zero sum

    // Fill dp table
    for (leaf_idx, leaf) in sorted_leaves.iter().enumerate() {
        // Consider all amounts we can currently achieve
        let current_amounts: Vec<u64> = dp.keys().cloned().collect();

        for &current_amount in &current_amounts {
            let new_amount = current_amount + leaf.value;

            // If this new amount doesn't exceed our target and we haven't found a way to achieve it yet
            if new_amount <= target_amount_sat && !dp.contains_key(&new_amount) {
                dp.insert(new_amount, (leaf_idx, current_amount));
            }
        }

        // Early exit if we've found our target
        if dp.contains_key(&target_amount_sat) {
            break;
        }
    }

    // If target amount cannot be reached
    if !dp.contains_key(&target_amount_sat) {
        return None;
    }

    // Reconstruct the solution by backtracking through the dp table
    let mut result = Vec::new();
    let mut current_amount = target_amount_sat;

    while current_amount > 0 {
        let (leaf_idx, prev_amount) = *dp.get(&current_amount).unwrap();
        if leaf_idx == usize::MAX {
            break; // Reached the special zero marker
        }
        result.push(sorted_leaves[leaf_idx].clone());
        current_amount = prev_amount;
    }

    Some(result)
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
            if let Err(e) = tree_service.finalize_reservation(leaves.id.clone()).await {
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
