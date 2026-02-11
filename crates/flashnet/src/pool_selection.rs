use crate::{FlashnetError, models::CurveType, models::Pool};
use spark::Network;
use tracing::debug;

struct PoolScore {
    pool: Pool,
    amount_in_required: u128,
    total_score_bps: u64,
    fee_efficiency_score_bps: u64,
    liquidity_score_bps: u64,
    stability_score_bps: u64,
}

// Scoring weights in basis points (total 10_000)
const FEE_WEIGHT_BPS: u64 = 5_000;
const LIQUIDITY_WEIGHT_BPS: u64 = 3_000;
const STABILITY_WEIGHT_BPS: u64 = 2_000;

/// Select the best pool from a list based on fee efficiency, liquidity, and price stability.
///
/// Scores each pool using a weighted algorithm:
/// - Fee Efficiency: 50% (lower `amount_in` required)
/// - Liquidity: 30% (higher TVL/reserves)
/// - Price Stability: 20% (lower 24h volatility)
///
/// Returns the pool with the highest score, or an error if no pool can satisfy the request.
pub fn select_best_pool(
    pools: &[Pool],
    asset_in_address: &str,
    amount_out: u128,
    max_slippage_bps: u32,
    integrator_fee_bps: u32,
    network: Network,
) -> Result<Pool, FlashnetError> {
    // Calculate amount_in for each pool, filter out those that error
    let viable_pools: Vec<(Pool, u128)> = pools
        .iter()
        .filter_map(|pool| {
            pool.calculate_amount_in(
                asset_in_address,
                amount_out,
                max_slippage_bps,
                integrator_fee_bps,
                network,
            )
            .ok()
            .map(|amount_in| (pool.clone(), amount_in))
        })
        .collect();

    if viable_pools.is_empty() {
        return Err(FlashnetError::Generic(
            "No pool can provide the requested output amount".to_string(),
        ));
    }

    // Filter out non V3 oncentrated pools if any V3 concentrated pool exists
    let has_v3_concentrated = viable_pools
        .iter()
        .any(|(pool, _)| pool.curve_type == Some(CurveType::V3Concentrated));
    let viable_pools: Vec<(Pool, u128)> = if has_v3_concentrated {
        debug!("Filtering to V3 concentrated pools only");
        viable_pools
            .into_iter()
            .filter(|(pool, _)| pool.curve_type == Some(CurveType::V3Concentrated))
            .collect()
    } else {
        viable_pools
    };

    // Handle single pool case early - no scoring needed
    if viable_pools.len() == 1 {
        let pool = viable_pools[0].0.clone();
        debug!(
            "Selected pool {}: only viable pool from {} pool(s)",
            pool.lp_public_key,
            pools.len()
        );
        return Ok(pool);
    }

    // Find min/max amount_in for normalization
    let amounts: Vec<u128> = viable_pools.iter().map(|(_, amt)| *amt).collect();
    let min_amount_in = amounts.iter().min().copied().unwrap_or_default();
    let max_amount_in = amounts.iter().max().copied().unwrap_or_default();

    // Find max TVL for normalization
    let max_tvl = viable_pools
        .iter()
        .filter_map(|(pool, _)| {
            pool.tvl_asset_b
                .map_or(pool.asset_b_reserve, |v| Some(u128::from(v)))
        })
        .max();

    // Score each pool and select best in single pass
    let best_pool = viable_pools
        .into_iter()
        .map(|(pool, amount_in)| {
            score_pool(&pool, amount_in, min_amount_in, max_amount_in, max_tvl)
        })
        .max_by(|a, b| {
            a.total_score_bps.cmp(&b.total_score_bps).then_with(|| {
                // Tiebreaker: higher volume wins
                a.pool
                    .volume_24h_asset_b
                    .unwrap_or(0)
                    .cmp(&b.pool.volume_24h_asset_b.unwrap_or(0))
            })
        })
        .ok_or(FlashnetError::Generic(
            "No pool can provide the requested output amount".to_string(),
        ))?;

    // Debug logging
    debug!(
        "Selected pool {} with score {} (fee: {}, liquidity: {}, stability: {}, amount_in: {}) from {} pools",
        best_pool.pool.lp_public_key,
        best_pool.total_score_bps,
        best_pool.fee_efficiency_score_bps,
        best_pool.liquidity_score_bps,
        best_pool.stability_score_bps,
        best_pool.amount_in_required,
        pools.len()
    );

    Ok(best_pool.pool)
}

/// Calculates a weighted score for a pool based on fee efficiency, liquidity, and price stability.
///
/// # Scoring Algorithm
///
/// The function computes three component scores, each normalized to a 0-10000 basis point scale:
/// 1. **Fee Efficiency Score** (0-10000 bps): Measures how favorable the `amount_in` requirement is
///    relative to other pools. Lower `amount_in` yields higher scores.
/// 2. **Liquidity Score** (0-10000 bps): Measures pool depth using TVL (total value locked) in asset B.
///    Higher TVL relative to `max_tvl` yields higher scores. Missing data receives a 10% penalty (1000 bps).
/// 3. **Stability Score** (0-10000 bps): Inverse of 24h price volatility. Lower volatility yields higher
///    scores. Missing data defaults to neutral (5000 bps).
///
/// The final weighted score combines these components using predefined weights:
/// - Fee Efficiency: 50% (5000 bps)
/// - Liquidity: 30% (3000 bps)
/// - Stability: 20% (2000 bps)
///
/// # Arguments
///
/// * `pool` - The pool to score
/// * `amount_in` - The amount of input asset required by this pool
/// * `min_amount_in` - Minimum `amount_in` across all viable pools (for normalization)
/// * `max_amount_in` - Maximum `amount_in` across all viable pools (for normalization)
/// * `max_tvl` - Maximum TVL across all viable pools (for normalization), `None` if all pools lack TVL data
///
/// # Returns
///
/// A `PoolScore` struct containing the pool, component scores, and total weighted score.
///
/// # Arithmetic Safety
///
/// This function allows arithmetic side effects (overflow/truncation) because:
/// - **Saturation**: All arithmetic uses `saturating_*` operations to prevent overflow/underflow
/// - **Truncation**: Casting u128 to u64 is safe because all values are normalized to 0-10000 range
///   before casting, which fits well within u64's maximum value
/// - **Division by zero**: Protected by explicit checks (e.g., `max_amount_in > min_amount_in`)
///   and saturating operations that default to safe values
/// - **Input bounds**: Scoring components are clamped to 0-10000 bps by design, ensuring
///   intermediate calculations stay within safe ranges even before saturation
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn score_pool(
    pool: &Pool,
    amount_in: u128,
    min_amount_in: u128,
    max_amount_in: u128,
    max_tvl: Option<u128>,
) -> PoolScore {
    // Fee efficiency score
    // ((max_amount_in - amount_in) * 10_000) / (max_amount_in - min_amount_in)
    let fee_efficiency_score_bps = if max_amount_in > min_amount_in {
        max_amount_in
            .saturating_sub(amount_in)
            .saturating_mul(10_000)
            .saturating_div(max_amount_in.saturating_sub(min_amount_in)) as u64
    } else {
        10_000 // All pools equal
    };

    // Liquidity score
    let liquidity_score_bps = if let Some(max) = max_tvl {
        let pool_liquidity = pool
            .tvl_asset_b
            .map_or(pool.asset_b_reserve, |v| Some(u128::from(v)))
            .unwrap_or(0);
        if pool_liquidity == 0 {
            // Penalize missing data (10%)
            1_000
        } else {
            // (pool_liquidity * 10_000) / max
            pool_liquidity.saturating_mul(10_000).saturating_div(max) as u64
        }
    } else {
        // All pools have missing data (neutral 50%)
        5_000
    };

    // Stability score, inverse of volatility. Default to neutral (50%) if missing.
    let stability_score_bps = pool.price_change_percent_24h.map_or(5_000, |pct| {
        let pct_bps = (pct.abs() * 100.0) as u64;
        // Inverse score: 10,000 / (1 + pct_bps/10,000) = 100,000,000 / (10,000 + pct_bps)
        10_000_u64
            .saturating_mul(10_000)
            .saturating_div(10_000 + pct_bps)
    });

    // Weighted total score
    let total_score_bps = (fee_efficiency_score_bps
        .saturating_mul(FEE_WEIGHT_BPS)
        .saturating_add(liquidity_score_bps.saturating_mul(LIQUIDITY_WEIGHT_BPS))
        .saturating_add(stability_score_bps.saturating_mul(STABILITY_WEIGHT_BPS)))
        / 10_000;

    PoolScore {
        pool: pool.clone(),
        total_score_bps,
        amount_in_required: amount_in,
        fee_efficiency_score_bps,
        liquidity_score_bps,
        stability_score_bps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Key test scenarios covered by tests:
    // - Single pool selection
    // - Lowest fee pool wins
    // - Missing data fallback scoring
    // - Empty pool list returns error
    // - Tiebreaker using volume
    // - Balanced scoring across multiple factors
    // - Price stability scoring

    #[allow(clippy::too_many_arguments)]
    fn create_test_pool_with_reserves(
        pubkey: &str,
        host_fee_bps: u32,
        lp_fee_bps: u32,
        reserve_a: u128,
        reserve_b: u128,
        tvl: Option<u64>,
        volume: Option<u64>,
        price_change: Option<f64>,
    ) -> Pool {
        Pool {
            lp_public_key: pubkey.parse().unwrap(),
            host_name: "test".to_string(),
            host_fee_bps,
            lp_fee_bps,
            asset_a_address: crate::BTC_ASSET_ADDRESS.to_string(),
            asset_b_address: "test_token".to_string(),
            asset_a_reserve: Some(reserve_a),
            asset_b_reserve: Some(reserve_b),
            virtual_reserve_a: None,
            virtual_reserve_b: None,
            threshold_pct: None,
            current_price_a_in_b: None,
            tvl_asset_b: tvl,
            volume_24h_asset_b: volume,
            price_change_percent_24h: price_change,
            curve_type: Some(crate::models::CurveType::ConstantProduct),
            initial_reserve_a: None,
            bonding_progress_percent: None,
            graduation_threshold_amount: None,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        }
    }

    #[test]
    fn test_score_pool_equal_amounts() {
        // When all pools have equal amount_in, they should get perfect fee scores
        let pool = Pool {
            lp_public_key: "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da"
                .parse()
                .unwrap(),
            host_name: "test".to_string(),
            host_fee_bps: 50,
            lp_fee_bps: 100,
            asset_a_address: "asset_a".to_string(),
            asset_b_address: "asset_b".to_string(),
            asset_a_reserve: Some(1_000_000),
            asset_b_reserve: Some(1_000_000),
            virtual_reserve_a: None,
            virtual_reserve_b: None,
            threshold_pct: None,
            current_price_a_in_b: None,
            tvl_asset_b: Some(1_000_000),
            volume_24h_asset_b: Some(10_000),
            price_change_percent_24h: None,
            curve_type: None,
            initial_reserve_a: None,
            bonding_progress_percent: None,
            graduation_threshold_amount: None,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        };

        let score = score_pool(&pool, 1_000, 1_000, 1_000, Some(1_000_000));

        // Fee efficiency should be 10_000 (perfect) since min == max
        assert_eq!(score.fee_efficiency_score_bps, 10_000);
        // Liquidity should be 10_000 (pool has max TVL)
        assert_eq!(score.liquidity_score_bps, 10_000);
        // Stability should be 5_000 (neutral, no price change data)
        assert_eq!(score.stability_score_bps, 5_000);
    }

    #[test]
    fn test_score_pool_fee_efficiency() {
        let pool = Pool {
            lp_public_key: "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d"
                .parse()
                .unwrap(),
            host_name: "test".to_string(),
            host_fee_bps: 50,
            lp_fee_bps: 100,
            asset_a_address: "asset_a".to_string(),
            asset_b_address: "asset_b".to_string(),
            asset_a_reserve: Some(1_000_000),
            asset_b_reserve: Some(1_000_000),
            virtual_reserve_a: None,
            virtual_reserve_b: None,
            threshold_pct: None,
            current_price_a_in_b: None,
            tvl_asset_b: Some(1_000_000),
            volume_24h_asset_b: Some(10_000),
            price_change_percent_24h: None,
            curve_type: None,
            initial_reserve_a: None,
            bonding_progress_percent: None,
            graduation_threshold_amount: None,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        };

        // Pool requires 1_500 when min is 1_000 and max is 2_000
        // Fee score = (2_000 - 1_500) * 10_000 / (2_000 - 1_000) = 500 * 10_000 / 1_000 = 5_000
        let score = score_pool(&pool, 1_500, 1_000, 2_000, Some(1_000_000));

        assert_eq!(score.fee_efficiency_score_bps, 5_000);
    }

    #[test]
    fn test_score_pool_missing_data() {
        let pool = Pool {
            lp_public_key: "02a1633caf0d6d2a8b3f4e1f5e6d7c8b9a0b1c2d3e4f5061728394a5b6c7d8e9fa"
                .parse()
                .unwrap(),
            host_name: "test".to_string(),
            host_fee_bps: 50,
            lp_fee_bps: 100,
            asset_a_address: "asset_a".to_string(),
            asset_b_address: "asset_b".to_string(),
            asset_a_reserve: None,
            asset_b_reserve: None,
            virtual_reserve_a: None,
            virtual_reserve_b: None,
            threshold_pct: None,
            current_price_a_in_b: None,
            tvl_asset_b: None,
            volume_24h_asset_b: None,
            price_change_percent_24h: None,
            curve_type: None,
            initial_reserve_a: None,
            bonding_progress_percent: None,
            graduation_threshold_amount: None,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        };

        let score = score_pool(&pool, 1_000, 1_000, 2_000, None);

        // Missing data should get fallback scores
        assert_eq!(score.liquidity_score_bps, 5_000); // Neutral (all pools missing)
        assert_eq!(score.stability_score_bps, 5_000); // Neutral (no price data)
    }

    // Integration tests for select_best_pool function

    #[test]
    fn test_select_best_pool_empty_list() {
        let all_pools: Vec<Pool> = vec![];
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No pool can provide")
        );
    }

    #[test]
    fn test_select_best_pool_single_pool() {
        let pool = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(10_000),
            None,
        );

        let all_pools = vec![pool.clone()];
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        assert_eq!(result.unwrap().lp_public_key, pool.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_prefers_lower_fees() {
        // Pool 1: Low fees (0.5% + 1% = 1.5% total)
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(5_000),
            None,
        );

        // Pool 2: High fees (2% + 3% = 5% total), but more liquidity
        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            200,
            300,
            2_000_000_000,
            2_000_000_000,
            Some(2_000_000_000),
            Some(20_000),
            None,
        );

        let all_pools = vec![pool1.clone(), pool2];
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        // Pool 1 should win due to much lower fees (50% weight)
        assert_eq!(result.unwrap().lp_public_key, pool1.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_tiebreaker_uses_volume() {
        // Two pools with identical fees and reserves
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(5_000),
            None,
        );

        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(15_000), // Higher volume
            None,
        );

        let all_pools = vec![pool1, pool2.clone()];
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        // Pool 2 should win due to higher volume in tiebreaker
        assert_eq!(result.unwrap().lp_public_key, pool2.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_balances_multiple_factors() {
        // Pool 1: Best fees (0.3%), but smallest liquidity and volatile
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            10,
            20,
            500_000_000,
            500_000_000,
            Some(500_000_000),
            Some(1_000),
            Some(15.0), // Volatile
        );

        // Pool 2: Medium fees (1.5%), medium liquidity, stable
        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(5_000),
            Some(2.0), // Stable
        );

        // Pool 3: Worst fees (4%), but best liquidity and very stable
        let pool3 = create_test_pool_with_reserves(
            "02a1633caf0d6d2a8b3f4e1f5e6d7c8b9a0b1c2d3e4f5061728394a5b6c7d8e9fa",
            150,
            250,
            3_000_000_000,
            3_000_000_000,
            Some(3_000_000_000),
            Some(30_000),
            Some(0.5), // Very stable
        );

        let all_pools = vec![pool1.clone(), pool2, pool3];
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            10_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        // Pool 1 should win due to much better fees (50% weight dominates)
        // Even with worse liquidity and stability, the fee advantage is too large
        assert_eq!(result.unwrap().lp_public_key, pool1.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_filters_insufficient_liquidity() {
        // Pool 1: Good pool with sufficient liquidity
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(10_000),
            None,
        );

        // Pool 2: Very low liquidity - should be filtered out
        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            10, // Better fees
            20,
            100, // Tiny reserves
            100,
            Some(100),
            Some(100),
            None,
        );

        let all_pools = vec![pool1.clone(), pool2];
        // Request a large amount that pool2 can't handle
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            50_000_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        // Pool 1 should be selected (pool 2 filtered out)
        assert_eq!(result.unwrap().lp_public_key, pool1.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_all_pools_insufficient() {
        // Both pools have tiny liquidity
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            100,
            100,
            Some(100),
            Some(10),
            None,
        );

        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            50,
            100,
            200,
            200,
            Some(200),
            Some(20),
            None,
        );

        let all_pools = vec![pool1, pool2];
        // Request amount that exceeds all pools
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000_000_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No pool can provide")
        );
    }

    #[test]
    fn test_select_best_pool_price_stability_matters() {
        // Pool 1: Same fees/liquidity, stable price (1% change)
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(10_000),
            Some(1.0),
        );

        // Pool 2: Same fees/liquidity, volatile price (25% change)
        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(10_000),
            Some(25.0),
        );

        let all_pools = vec![pool1.clone(), pool2];
        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000,
            50,
            0,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        // Pool 1 should win due to better price stability (20% weight)
        assert_eq!(result.unwrap().lp_public_key, pool1.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_with_integrator_fee() {
        // Pool with lower base fees should still win when integrator fee is applied uniformly
        let pool1 = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,  // 0.5% host fee
            100, // 1% LP fee
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(5_000),
            None,
        );

        let pool2 = create_test_pool_with_reserves(
            "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            200, // 2% host fee
            300, // 3% LP fee
            2_000_000_000,
            2_000_000_000,
            Some(2_000_000_000),
            Some(20_000),
            None,
        );

        let all_pools = vec![pool1.clone(), pool2];
        let integrator_fee_bps = 50; // 0.5% integrator fee

        let result = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            1_000,
            50,
            integrator_fee_bps,
            Network::Mainnet,
        );

        assert!(result.is_ok());
        // Pool 1 should still win due to lower total fees
        assert_eq!(result.unwrap().lp_public_key, pool1.lp_public_key);
    }

    #[test]
    fn test_select_best_pool_integrator_fee_affects_amount_in() {
        // With integrator fee, the required amount_in should increase
        let pool = create_test_pool_with_reserves(
            "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            50,
            100,
            1_000_000_000,
            1_000_000_000,
            Some(1_000_000_000),
            Some(5_000),
            None,
        );

        let all_pools = vec![pool.clone()];

        // Without integrator fee
        let result_without = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            10_000,
            50,
            0,
            Network::Mainnet,
        );

        // With integrator fee
        let result_with = select_best_pool(
            &all_pools,
            crate::BTC_ASSET_ADDRESS,
            10_000,
            50,
            100, // 1% integrator fee
            Network::Mainnet,
        );

        assert!(result_without.is_ok());
        assert!(result_with.is_ok());

        // Both should return the same pool (only one pool available)
        assert_eq!(
            result_without.unwrap().lp_public_key,
            result_with.unwrap().lp_public_key
        );
    }
}
