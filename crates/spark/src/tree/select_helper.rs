use tracing::{error, trace};

use crate::tree::{
    LeafLike, LeavesReservation, TargetAmounts, TargetLeaves, TreeService, TreeServiceError,
};

pub fn select_leaves_by_target_amounts<L: LeafLike>(
    leaves: &[L],
    target_amounts: Option<&TargetAmounts>,
) -> Result<TargetLeaves<L>, TreeServiceError> {
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
                            .any(|amount_leaf| amount_leaf.leaf_id() == leaf.leaf_id())
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
pub fn select_leaves_by_exact_amount<L: LeafLike>(
    leaves: &[L],
    target_amount_sat: u64,
) -> Result<Option<Vec<L>>, TreeServiceError> {
    if target_amount_sat == 0 {
        return Err(TreeServiceError::InvalidAmount);
    }

    if leaves.iter().map(LeafLike::leaf_value).sum::<u64>() < target_amount_sat {
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

/// Splits `leaves` into one group per amount in `amounts`, each summing to it
/// exactly, in the order of `amounts`. `None` when the leaves cannot be split
/// that way. Larger amounts are filled first, which is what leaves the small
/// ones a chance among the leftovers.
pub fn partition_by_exact_amounts<L: LeafLike>(
    leaves: &[L],
    amounts: &[u64],
) -> Option<Vec<Vec<L>>> {
    let total: u64 = leaves.iter().map(LeafLike::leaf_value).sum();
    if amounts.iter().copied().fold(0u64, u64::saturating_add) != total {
        return None;
    }
    let mut order: Vec<usize> = (0..amounts.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(amounts.get(i).copied().unwrap_or_default()));

    let mut remaining = leaves.to_vec();
    let mut groups: Vec<Option<Vec<L>>> = vec![None; amounts.len()];
    for i in order {
        let amount = *amounts.get(i)?;
        let group = select_leaves_by_exact_amount(&remaining, amount).ok()??;
        remaining.retain(|leaf| !group.iter().any(|g| g.leaf_id() == leaf.leaf_id()));
        *groups.get_mut(i)? = Some(group);
    }
    groups.into_iter().collect()
}

pub fn select_leaves_by_exact_denominations<L: LeafLike>(
    leaves: &[L],
    denominations: &[u64],
) -> Result<Vec<L>, TreeServiceError> {
    let mut remaining_leaves = leaves.to_vec();
    let mut selected_leaves = Vec::new();

    for denomination in denominations {
        let idx = remaining_leaves
            .iter()
            .position(|l| l.leaf_value() == *denomination)
            .ok_or(TreeServiceError::UnselectableAmount)?;
        selected_leaves.push(remaining_leaves.swap_remove(idx));
    }

    Ok(selected_leaves)
}

/// Selects leaves from the tree that sum up to at least the target amount.
pub fn select_leaves_by_minimum_amount<L: LeafLike>(
    leaves: &[L],
    target_amount_sat: u64,
) -> Result<Option<Vec<L>>, TreeServiceError> {
    if target_amount_sat == 0 {
        return Err(TreeServiceError::InvalidAmount);
    }
    if leaves.iter().map(LeafLike::leaf_value).sum::<u64>() < target_amount_sat {
        return Err(TreeServiceError::InsufficientFunds);
    }

    let mut result = Vec::new();
    let mut sum = 0;
    for leaf in leaves {
        sum += leaf.leaf_value();
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

pub(crate) fn find_exact_single_match<L: LeafLike>(
    leaves: &[L],
    target_amount_sat: u64,
) -> Option<L> {
    leaves
        .iter()
        .find(|leaf| leaf.leaf_value() == target_amount_sat)
        .cloned()
}

/// Helper to check if a value is a power of two
fn is_power_of_two(value: u64) -> bool {
    value > 0 && (value & (value - 1)) == 0
}

/// Greedy algorithm to find exact match.
/// Sorts leaves by value in descending order and takes the largest leaf that fits
/// the remaining amount until the target is reached or no valid leaf can be found.
fn greedy_exact_match<L: LeafLike>(leaves: &[L], target_amount_sat: u64) -> Option<Vec<L>> {
    let mut sorted_leaves = leaves.to_vec();
    sorted_leaves.sort_by_key(|b| std::cmp::Reverse(b.leaf_value()));

    let mut result = Vec::new();
    let mut remaining = target_amount_sat;

    for leaf in &sorted_leaves {
        if leaf.leaf_value() > remaining {
            continue;
        }
        remaining -= leaf.leaf_value();
        result.push(leaf.clone());
        if remaining == 0 {
            return Some(result);
        }
    }

    None // Couldn't reach exact target
}

pub(crate) fn find_exact_multiple_match<L: LeafLike>(
    leaves: &[L],
    target_amount_sat: u64,
) -> Option<Vec<L>> {
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
    let power_of_two_leaves: Vec<L> = leaves
        .iter()
        .filter(|l| is_power_of_two(l.leaf_value()))
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
            if let Err(e) = tree_service.cancel_reservation(leaves.clone()).await {
                error!("Failed to cancel reservation: {e:?}");
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod partition_tests {
    use macros::test_all;

    use super::partition_by_exact_amounts;
    use crate::tree::TreeNode;
    use crate::tree::tests::create_test_tree_node;

    fn leaves(values: &[u64]) -> Vec<TreeNode> {
        values
            .iter()
            .enumerate()
            .map(|(i, value)| create_test_tree_node(&format!("leaf-{i}"), *value))
            .collect()
    }

    fn sums(groups: &[Vec<TreeNode>]) -> Vec<u64> {
        groups
            .iter()
            .map(|group| group.iter().map(|leaf| leaf.value).sum())
            .collect()
    }

    #[test_all]
    fn splits_leaves_into_groups_in_the_order_asked() {
        let groups =
            partition_by_exact_amounts(&leaves(&[8_192, 2_048, 256, 64, 16, 4]), &[64, 10_496, 20])
                .unwrap();
        assert_eq!(sums(&groups), vec![64, 10_496, 20]);
        let used: usize = groups.iter().map(Vec::len).sum();
        assert_eq!(used, 6, "every leaf lands in exactly one group");
    }

    #[test_all]
    fn refuses_amounts_that_do_not_add_up_to_the_leaves() {
        assert!(partition_by_exact_amounts(&leaves(&[1_024, 512]), &[1_024]).is_none());
        assert!(partition_by_exact_amounts(&leaves(&[1_024]), &[1_024, 1]).is_none());
    }

    #[test_all]
    fn refuses_a_split_no_subset_makes() {
        // 3 + 3 cannot be carved out of a single 6-sat leaf.
        assert!(partition_by_exact_amounts(&leaves(&[6]), &[3, 3]).is_none());
    }
}
