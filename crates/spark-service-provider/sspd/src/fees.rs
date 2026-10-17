use std::error::Error;
use std::iter;

use bitcoin::transaction::{InputWeightPrediction, predict_weight};
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
pub fn fee_sats(fee_rate: FeeRate, weight: Weight) -> u64 {
    fee_rate
        .max(MIN_FEE_RATE)
        .fee_wu(weight)
        .map_or(u64::MAX, Amount::to_sat)
}

/// Element count, length byte and 64-byte Schnorr signature, at 1 WU per
/// witness byte.
pub const P2TR_KEY_PATH_WITNESS: Weight = Weight::from_wu(66);

/// Outpoint, empty scriptSig and sequence at 4 WU per byte, plus the witness.
pub const P2TR_INPUT: Weight = Weight::from_wu(41 * 4 + P2TR_KEY_PATH_WITNESS.to_wu());

/// Value, script length and 34-byte script. No common output type is larger, so
/// sizing with it is conservative for a destination not known in advance.
pub const P2TR_OUTPUT: Weight = Weight::from_wu(43 * 4);

pub const P2TR_SCRIPT_LEN: usize = 34;

pub const P2A_SCRIPT_LEN: usize = 4;

#[must_use]
pub fn p2tr_tx_weight(inputs: usize, outputs: usize) -> Weight {
    predict_weight(
        iter::repeat_n(InputWeightPrediction::P2TR_KEY_DEFAULT_SIGHASH, inputs),
        iter::repeat_n(P2TR_SCRIPT_LEN, outputs),
    )
}

/// The smallest P2TR output nodes relay.
pub const P2TR_DUST_SATS: u64 = 330;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_rounds_up_and_respects_the_floor() {
        assert_eq!(fee_sats(MIN_FEE_RATE, Weight::from_wu(1000)), 253);
        assert_eq!(fee_sats(FeeRate::ZERO, Weight::from_wu(1000)), 253);
        // 444 x 253 / 1000 = 112.332, rounded up.
        assert_eq!(
            fee_sats(FeeRate::from_sat_per_kwu(253), Weight::from_wu(444)),
            113
        );
    }
}
