use std::error::Error;

use bitcoin::{Amount, FeeRate, Weight};

type BoxError = Box<dyn Error + Send + Sync>;

/// Floor for any fee rate the SSP works from: the conventional 253 sat/kw, just
/// above 1 sat/vByte (250 sat/kw).
pub const MIN_FEE_RATE: FeeRate = FeeRate::from_sat_per_kwu(253);

#[async_trait::async_trait]
pub trait FeeRateSource: Send + Sync {
    /// The returned rate is never below [`MIN_FEE_RATE`].
    async fn fee_rate(&self) -> Result<FeeRate, BoxError>;
}

/// Rounded up, since a fee rounded down can leave the transaction below the rate
/// it was calculated at.
#[must_use]
pub fn fee_sats(fee_rate: FeeRate, weight_wu: u64) -> u64 {
    fee_rate
        .max(MIN_FEE_RATE)
        .fee_wu(Weight::from_wu(weight_wu))
        .map_or(u64::MAX, Amount::to_sat)
}

/// Element count, length byte and 64-byte Schnorr signature, at 1 WU per
/// witness byte.
pub const P2TR_KEY_PATH_WITNESS_WU: u64 = 66;

/// Version, lock time and the input and output counts at 4 WU per byte, plus the
/// segwit marker and flag at 1 WU each.
pub const TX_OVERHEAD_WU: u64 = 42;

/// Outpoint, empty scriptSig and sequence at 4 WU per byte, plus the witness.
pub const P2TR_INPUT_WU: u64 = 41 * 4 + P2TR_KEY_PATH_WITNESS_WU;

/// Value, script length and 34-byte script. No common output type is larger, so
/// sizing with it is conservative for a destination not known in advance.
pub const P2TR_OUTPUT_WU: u64 = 43 * 4;

/// Value, script length and 4-byte script.
pub const P2A_OUTPUT_WU: u64 = 13 * 4;

/// The smallest P2TR output nodes relay.
pub const P2TR_DUST_SATS: u64 = 330;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_rounds_up_and_respects_the_floor() {
        assert_eq!(fee_sats(MIN_FEE_RATE, 1000), 253);
        assert_eq!(fee_sats(FeeRate::ZERO, 1000), 253);
        // 444 x 253 / 1000 = 112.332, rounded up.
        assert_eq!(fee_sats(FeeRate::from_sat_per_kwu(253), 444), 113);
    }
}
