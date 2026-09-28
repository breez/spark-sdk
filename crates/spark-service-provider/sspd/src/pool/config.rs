use std::time::Duration;

/// Denominations below this get deeper trees. A tree is funded by one output worth
/// `denomination * leaves`, which the smallest denominations would otherwise make
/// dust.
const LARGE_DENOMINATION_THRESHOLD: u64 = 8192;
const SMALL_TREE_DEPTH: usize = 10;
const LARGE_TREE_DEPTH: usize = 4;

pub const BRANCH_FACTOR: usize = 2;

#[derive(Clone, Debug)]
pub struct PoolConfig {
    /// Target number of leaves of each denomination.
    pub leaves_per_denomination: u32,
    pub max_denomination_power: u32,
    pub replenish_interval: Duration,
}

impl PoolConfig {
    pub fn denominations(&self) -> Vec<u64> {
        (0..=self.max_denomination_power)
            .map(|i| 1u64 << i)
            .collect()
    }

    pub fn largest_denomination(&self) -> u64 {
        1u64 << self.max_denomination_power
    }

    pub fn tree_depth(denomination: u64) -> usize {
        if denomination < LARGE_DENOMINATION_THRESHOLD {
            SMALL_TREE_DEPTH
        } else {
            LARGE_TREE_DEPTH
        }
    }

    pub fn leaves_per_tree(denomination: u64) -> usize {
        1 << Self::tree_depth(denomination)
    }

    pub fn tree_value(denomination: u64) -> u64 {
        denomination.saturating_mul(Self::leaves_per_tree(denomination) as u64)
    }
}
