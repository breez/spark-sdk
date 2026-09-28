/// Largest first, none above `largest`.
pub fn decompose_into_powers_of_two(mut amount: u64, largest: u64) -> Vec<u64> {
    let mut result = Vec::new();
    while amount > 0 {
        let leaf = (1u64 << amount.ilog2()).min(largest);
        result.push(leaf);
        amount = amount.saturating_sub(leaf);
    }
    result
}

/// Each target is decomposed on its own, so every target can be made up exactly
/// from a separate subset of the leaves: decomposing the total can yield a leaf
/// larger than a target (17232 yields 16384, more than a 15000 target).
pub fn swap_denominations(targets: &[u64], amount_to_send: u64, largest: u64) -> Option<Vec<u64>> {
    let target_sum: u64 = targets.iter().sum();
    let change = amount_to_send.checked_sub(target_sum)?;

    let mut denominations = Vec::new();
    for &target in targets {
        denominations.extend(decompose_into_powers_of_two(target, largest));
    }
    if change > 0 {
        denominations.extend(decompose_into_powers_of_two(change, largest));
    }
    Some(denominations)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LARGEST: u64 = 1 << 20;

    #[test]
    fn test_decompose_powers_of_two() {
        assert_eq!(decompose_into_powers_of_two(7, LARGEST), vec![4, 2, 1]);
        assert_eq!(decompose_into_powers_of_two(10, LARGEST), vec![8, 2]);
        assert_eq!(decompose_into_powers_of_two(1, LARGEST), vec![1]);
        assert_eq!(decompose_into_powers_of_two(8, LARGEST), vec![8]);
        assert_eq!(decompose_into_powers_of_two(15, LARGEST), vec![8, 4, 2, 1]);
        assert_eq!(
            decompose_into_powers_of_two(1023, LARGEST),
            vec![512, 256, 128, 64, 32, 16, 8, 4, 2, 1]
        );
    }

    #[test]
    fn test_decompose_caps_at_largest() {
        assert_eq!(decompose_into_powers_of_two(20, 4), vec![4, 4, 4, 4, 4]);
        assert_eq!(decompose_into_powers_of_two(11, 4), vec![4, 4, 2, 1]);
    }

    #[test]
    fn test_swap_denominations_per_target() {
        let denominations = swap_denominations(&[15000, 2232], 17232, LARGEST).unwrap();
        assert_eq!(denominations.iter().sum::<u64>(), 17232);

        assert_eq!(
            decompose_into_powers_of_two(15000, LARGEST),
            vec![8192, 4096, 2048, 512, 128, 16, 8]
        );
        assert_eq!(
            decompose_into_powers_of_two(2232, LARGEST),
            vec![2048, 128, 32, 16, 8]
        );

        assert!(!denominations.contains(&16384));
    }

    #[test]
    fn test_swap_denominations_with_change() {
        let denominations = swap_denominations(&[100], 130, LARGEST).unwrap();
        assert_eq!(denominations.iter().sum::<u64>(), 130);
        assert_eq!(denominations, vec![64, 32, 4, 16, 8, 4, 2]);
    }

    #[test]
    fn test_swap_denominations_rejects_targets_over_send() {
        assert!(swap_denominations(&[100, 50], 120, LARGEST).is_none());
    }

    #[test]
    fn test_swap_denominations_empty_targets() {
        assert_eq!(swap_denominations(&[], 10, LARGEST), Some(vec![8, 2]));
    }
}
