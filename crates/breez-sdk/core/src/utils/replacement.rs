use bitcoin::Transaction;
use spark_wallet::MIN_RELAY_FEE_SAT_PER_VBYTE;

/// A transaction on the network that a replacement has to displace. Its size
/// matters as well as its fee: the replacement has to beat its feerate, not just
/// its total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Replaced {
    pub(crate) fee_sats: u64,
    pub(crate) vsize: u64,
}

impl Replaced {
    /// Describes `tx`, which spends inputs worth `input_value_sats`. `None` if
    /// it pays out more than that.
    pub(crate) fn spending(tx: &Transaction, input_value_sats: u64) -> Option<Self> {
        Some(Self {
            fee_sats: fee_paid_sats(tx, input_value_sats)?,
            vsize: tx.vsize().try_into().unwrap_or(u64::MAX),
        })
    }

    /// Minimum fee a replacement of `vsize` vbytes must pay to displace this
    /// transaction: more than it pays, plus the relay cost of its own size.
    pub(crate) fn min_replacement_fee_sats(&self, vsize: u64) -> u64 {
        // Cover the replaced fee plus the replacement's own relay bandwidth.
        let bandwidth = self
            .fee_sats
            .saturating_add(vsize.saturating_mul(MIN_RELAY_FEE_SAT_PER_VBYTE));
        // And beat its feerate outright, which only bites when the replacement is
        // the larger transaction, as it is when the destination widens to taproot.
        // The comparison is made on feerates truncated to whole sat/kvB, so a fee
        // that is higher as an exact rational can still land in the same bucket and
        // be refused.
        let replaced_per_kvb = self
            .fee_sats
            .saturating_mul(1000)
            .checked_div(self.vsize)
            .unwrap_or(u64::MAX);
        let feerate = replaced_per_kvb
            .saturating_add(1)
            .saturating_mul(vsize)
            .div_ceil(1000);
        bandwidth.max(feerate)
    }
}

/// The fee `tx` pays out of inputs worth `input_value_sats`. `None` if it pays
/// out more.
pub(crate) fn fee_paid_sats(tx: &Transaction, input_value_sats: u64) -> Option<u64> {
    let out_sats: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
    input_value_sats.checked_sub(out_sats)
}

#[cfg(test)]
mod tests {
    use bitcoin::{Amount, ScriptBuf, TxOut, absolute::LockTime, transaction::Version};

    use super::*;

    fn paying_out(sats: u64) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![TxOut {
                value: Amount::from_sat(sats),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    #[test]
    fn the_fee_is_what_the_outputs_leave_behind() {
        assert_eq!(fee_paid_sats(&paying_out(99_889), 100_000), Some(111));
        assert_eq!(fee_paid_sats(&paying_out(100_000), 100_000), Some(0));
        assert_eq!(fee_paid_sats(&paying_out(100_001), 100_000), None);
    }

    #[test]
    fn a_replacement_must_cover_the_replaced_fee_and_its_own_relay() {
        // Displacing a 111 sat transaction with a 111 vbyte replacement costs 222,
        // not 112: the replacement also pays to relay its own bytes.
        let replaced = Replaced {
            fee_sats: 111,
            vsize: 111,
        };
        assert_eq!(replaced.min_replacement_fee_sats(111), 222);
        let free = Replaced {
            fee_sats: 0,
            vsize: 111,
        };
        assert_eq!(free.min_replacement_fee_sats(111), 111);
    }

    #[test]
    fn a_larger_replacement_must_beat_the_replaced_feerate_too() {
        // Covering the replaced fee plus the replacement's own bandwidth is not
        // enough when the replacement is bigger: 3000 sats over 99 vB is 30.3
        // sat/vB, and 3111 over 111 vB would be 28.0, which the network refuses.
        let replaced = Replaced {
            fee_sats: 3_000,
            vsize: 99,
        };
        let required = replaced.min_replacement_fee_sats(111);
        assert_eq!(required, 3_364);
        assert!(
            required * replaced.vsize > replaced.fee_sats * 111,
            "a replacement has to beat the replaced feerate outright"
        );

        // Same size, so paying the bandwidth is all it takes.
        assert_eq!(replaced.min_replacement_fee_sats(99), 3_099);

        // Truncation to whole sat/kvB: 1045 over 111 vB and 932 over 99 both come
        // to 9414, which is a tie and refused, so the floor has to be 1046.
        let tie = Replaced {
            fee_sats: 932,
            vsize: 99,
        };
        assert_eq!(tie.min_replacement_fee_sats(111), 1_046);

        // A small replaced fee never reaches the feerate rule.
        let small = Replaced {
            fee_sats: 300,
            vsize: 99,
        };
        assert_eq!(small.min_replacement_fee_sats(111), 411);
    }
}
