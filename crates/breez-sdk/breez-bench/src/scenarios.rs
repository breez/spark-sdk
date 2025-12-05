//! Benchmark scenario definitions with seeded random and deterministic patterns.

use rand::Rng;
use rand_chacha::ChaCha8Rng;
use std::time::Duration;

/// Default seed for reproducible benchmarks.
pub const DEFAULT_SEED: u64 = 12345;

/// Default number of payments per benchmark run.
pub const DEFAULT_PAYMENT_COUNT: usize = 100;

/// Default amount range in satoshis.
/// Kept small to allow many payments within a single 50k deposit.
pub const DEFAULT_MIN_AMOUNT: u64 = 100;
pub const DEFAULT_MAX_AMOUNT: u64 = 2_000;

/// Default delay range between payments in milliseconds.
pub const DEFAULT_MIN_DELAY_MS: u64 = 500;
pub const DEFAULT_MAX_DELAY_MS: u64 = 3000;

/// Maximum initial funding amount (single deposit limit).
pub const MAX_INITIAL_FUNDING: u64 = 50_000;

/// How often the receiver sends funds back to sender (every N payments).
/// This simulates realistic bidirectional usage.
pub const DEFAULT_RETURN_INTERVAL: usize = 5;

/// A single payment to execute in the benchmark.
#[derive(Debug, Clone)]
pub struct PaymentSpec {
    /// Amount to send in satoshis
    pub amount_sats: u64,
    /// Delay before this payment
    pub delay: Duration,
}

/// Configuration for generating payment scenarios.
#[derive(Debug, Clone)]
pub struct ScenarioConfig {
    pub seed: u64,
    pub payment_count: usize,
    pub min_amount: u64,
    pub max_amount: u64,
    pub min_delay_ms: u64,
    pub max_delay_ms: u64,
    /// How often receiver sends funds back to sender (every N payments).
    /// Set to 0 to disable return payments.
    pub return_interval: usize,
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            payment_count: DEFAULT_PAYMENT_COUNT,
            min_amount: DEFAULT_MIN_AMOUNT,
            max_amount: DEFAULT_MAX_AMOUNT,
            min_delay_ms: DEFAULT_MIN_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
            return_interval: DEFAULT_RETURN_INTERVAL,
        }
    }
}

impl ScenarioConfig {
    /// Validate that the config is compatible with the max initial funding limit.
    pub fn validate(&self) -> Result<(), String> {
        // Rough estimate: we need enough buffer for at least return_interval payments
        // before receiving funds back
        let min_buffer = self.max_amount * self.return_interval.max(1) as u64;
        if min_buffer > MAX_INITIAL_FUNDING {
            return Err(format!(
                "max_amount ({}) * return_interval ({}) = {} exceeds MAX_INITIAL_FUNDING ({}). \
                 Reduce max_amount or return_interval.",
                self.max_amount, self.return_interval, min_buffer, MAX_INITIAL_FUNDING
            ));
        }
        Ok(())
    }
}

/// Named scenario presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioPreset {
    /// Seeded random payments (default)
    Random,
    /// Deterministic edge cases that test specific leaf configurations
    EdgeCases,
    /// Small payments only (likely no swaps needed)
    SmallPayments,
    /// Large payments only (more likely to need swaps)
    LargePayments,
}

impl ScenarioPreset {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "random" => Some(Self::Random),
            "edge-cases" | "edge_cases" | "edgecases" => Some(Self::EdgeCases),
            "small" | "small-payments" => Some(Self::SmallPayments),
            "large" | "large-payments" => Some(Self::LargePayments),
            _ => None,
        }
    }
}

/// Generate payment specifications for a benchmark run.
pub fn generate_payments(config: &ScenarioConfig, preset: ScenarioPreset) -> Vec<PaymentSpec> {
    match preset {
        ScenarioPreset::Random => generate_random_payments(config),
        ScenarioPreset::EdgeCases => generate_edge_case_payments(config),
        ScenarioPreset::SmallPayments => generate_small_payments(config),
        ScenarioPreset::LargePayments => generate_large_payments(config),
    }
}

/// Generate seeded pseudo-random payments.
fn generate_random_payments(config: &ScenarioConfig) -> Vec<PaymentSpec> {
    use rand::SeedableRng;

    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut payments = Vec::with_capacity(config.payment_count);

    for _ in 0..config.payment_count {
        let amount_sats = rng.gen_range(config.min_amount..=config.max_amount);
        let delay_ms = rng.gen_range(config.min_delay_ms..=config.max_delay_ms);

        payments.push(PaymentSpec {
            amount_sats,
            delay: Duration::from_millis(delay_ms),
        });
    }

    payments
}

/// Generate deterministic edge-case payments to test specific scenarios.
fn generate_edge_case_payments(config: &ScenarioConfig) -> Vec<PaymentSpec> {
    // Predefined amounts that exercise different leaf configurations
    let edge_amounts: Vec<u64> = vec![
        1,     // Minimum possible
        10,    // Very small
        100,   // Small
        256,   // Power of 2
        500,   // Round number
        1000,  // 1k sats
        1024,  // Power of 2
        2000,  // 2k sats
        4096,  // Power of 2
        5000,  // 5k sats
        7777,  // Odd number
        10000, // 10k sats
        12345, // Arbitrary
        20000, // 20k sats
        32768, // Power of 2
        50000, // 50k sats (if within range)
    ];

    // Filter to amounts within config range and repeat to fill count
    let valid_amounts: Vec<u64> = edge_amounts
        .into_iter()
        .filter(|&a| a >= config.min_amount && a <= config.max_amount)
        .collect();

    if valid_amounts.is_empty() {
        // Fall back to random if no edge cases fit the range
        return generate_random_payments(config);
    }

    let delay = Duration::from_millis((config.min_delay_ms + config.max_delay_ms) / 2);

    valid_amounts
        .into_iter()
        .cycle()
        .take(config.payment_count)
        .map(|amount_sats| PaymentSpec { amount_sats, delay })
        .collect()
}

/// Generate small payments (less likely to need swaps).
fn generate_small_payments(config: &ScenarioConfig) -> Vec<PaymentSpec> {
    let small_config = ScenarioConfig {
        min_amount: config.min_amount,
        max_amount: config
            .min_amount
            .saturating_add(1000)
            .min(config.max_amount),
        ..config.clone()
    };
    generate_random_payments(&small_config)
}

/// Generate large payments (more likely to need swaps).
fn generate_large_payments(config: &ScenarioConfig) -> Vec<PaymentSpec> {
    let large_config = ScenarioConfig {
        min_amount: config
            .max_amount
            .saturating_sub(10000)
            .max(config.min_amount),
        max_amount: config.max_amount,
        ..config.clone()
    };
    generate_random_payments(&large_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_payments_reproducible() {
        let config = ScenarioConfig {
            seed: 42,
            payment_count: 10,
            ..Default::default()
        };

        let payments1 = generate_random_payments(&config);
        let payments2 = generate_random_payments(&config);

        assert_eq!(payments1.len(), payments2.len());
        for (p1, p2) in payments1.iter().zip(payments2.iter()) {
            assert_eq!(p1.amount_sats, p2.amount_sats);
            assert_eq!(p1.delay, p2.delay);
        }
    }

    #[test]
    fn test_edge_case_payments() {
        let config = ScenarioConfig {
            payment_count: 20,
            min_amount: 100,
            max_amount: 10000,
            ..Default::default()
        };

        let payments = generate_edge_case_payments(&config);
        assert_eq!(payments.len(), 20);

        // All amounts should be within range
        for p in &payments {
            assert!(p.amount_sats >= config.min_amount);
            assert!(p.amount_sats <= config.max_amount);
        }
    }

    #[test]
    fn test_config_validation() {
        // Valid config
        let config = ScenarioConfig {
            max_amount: 2000,
            return_interval: 5,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Invalid config - would need too much buffer
        let invalid_config = ScenarioConfig {
            max_amount: 20000,
            return_interval: 5,
            ..Default::default()
        };
        // 20000 * 5 = 100000 > 50000, should fail
        assert!(invalid_config.validate().is_err());
    }
}
