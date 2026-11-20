use crate::error::SdkError;

/// Pure kernel for the `FeesIncluded` fee-reconciliation shared by the Bolt11,
/// LNURL-pay, and Boltz cross-chain send paths.
///
/// Given the fee stored at prepare time and the fee re-estimated at send time,
/// returns the allowed overpayment (`stored - current`). Fails if the fee
/// increased since prepare, or if the overpayment exceeds the cap of
/// `current_fee.max(1)` (allow up to 100% of the actual fee, minimum 1 sat).
pub(crate) fn fee_overpayment(stored_fee: u64, current_fee: u64) -> Result<u64, SdkError> {
    if current_fee > stored_fee {
        return Err(SdkError::Generic(
            "Fee increased since prepare. Please retry.".to_string(),
        ));
    }

    let overpayment = stored_fee.saturating_sub(current_fee);
    let max_allowed_overpayment = current_fee.max(1);
    if overpayment > max_allowed_overpayment {
        return Err(SdkError::Generic(format!(
            "Fee overpayment ({overpayment} sats) exceeds allowed maximum ({max_allowed_overpayment} sats)"
        )));
    }

    Ok(overpayment)
}

/// Whether `fee`, charged on `basis` sats, stays within the fee prepared on
/// `prepared_basis`. A fee can grow with the amount, so the prepared fee is
/// scaled up when the amount grew, never down: fixed parts don't shrink with
/// it. `rounding_sats` allows for each fee being rounded on its own.
pub(crate) fn fee_within_prepared(
    prepared_fee: u64,
    prepared_basis: u64,
    fee: u64,
    basis: u64,
    rounding_sats: u64,
) -> bool {
    let allowed = if prepared_basis == 0 {
        u128::from(prepared_fee)
    } else {
        u128::from(prepared_fee)
            .saturating_mul(u128::from(basis.max(prepared_basis)))
            .div_ceil(u128::from(prepared_basis))
    };
    u128::from(fee) <= allowed.saturating_add(u128::from(rounding_sats))
}

/// The sats a conversion estimated to deliver `amount_out` may fall short by.
pub(crate) fn slippage_margin_sats(amount_out: u128, max_slippage_bps: u32) -> u64 {
    let margin = amount_out
        .saturating_mul(u128::from(max_slippage_bps))
        .div_ceil(10_000);
    u64::try_from(margin).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{fee_overpayment, fee_within_prepared, slippage_margin_sats};
    use crate::error::SdkError;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_fee_overpayment_fee_decreased() {
        // Fee dropped from 100 → 60: overpayment is the 40 sat difference,
        // within the cap of current_fee.max(1) = 60.
        assert_eq!(fee_overpayment(100, 60).unwrap(), 40);
    }

    #[test_all]
    fn test_fee_overpayment_fee_unchanged() {
        assert_eq!(fee_overpayment(100, 100).unwrap(), 0);
    }

    #[test_all]
    fn test_fee_overpayment_fee_increased_fails() {
        let result = fee_overpayment(100, 101);
        assert!(result.is_err(), "Should fail when fee increased");
        if let Err(SdkError::Generic(msg)) = result {
            assert!(
                msg.contains("Fee increased since prepare"),
                "Error should mention fee increase"
            );
        } else {
            panic!("Expected Generic error");
        }
    }

    #[test_all]
    fn test_fee_overpayment_exceeds_cap_fails() {
        // current_fee = 1 → cap = max(1, 1) = 1, but overpayment = 100 - 1 = 99 > 1.
        let result = fee_overpayment(100, 1);
        assert!(result.is_err(), "Should fail when overpayment exceeds cap");
        if let Err(SdkError::Generic(msg)) = result {
            assert!(
                msg.contains("exceeds allowed maximum"),
                "Error should mention the cap"
            );
        } else {
            panic!("Expected Generic error");
        }
    }

    #[test_all]
    fn test_fee_overpayment_at_cap_succeeds() {
        // current_fee = 50 → cap = 50, overpayment = 100 - 50 = 50 == cap → allowed.
        assert_eq!(fee_overpayment(100, 50).unwrap(), 50);
    }

    #[test_all]
    fn test_fee_overpayment_zero_current_fee_min_cap() {
        // current_fee = 0 → cap = max(0, 1) = 1. stored_fee = 1 → overpayment 1 == cap.
        assert_eq!(fee_overpayment(1, 0).unwrap(), 1);
        // stored_fee = 2, current = 0 → overpayment 2 > cap 1 → fails.
        assert!(fee_overpayment(2, 0).is_err());
    }

    #[test_all]
    fn a_fee_on_a_smaller_amount_may_not_exceed_the_prepared_fee() {
        // 10 fixed + 1%: 1,010 on 100,000 prepared, 1,000 on 99,000 actual.
        assert!(fee_within_prepared(1_010, 100_000, 1_000, 99_000, 0));
        assert!(fee_within_prepared(1_010, 100_000, 1_010, 99_000, 0));
        assert!(!fee_within_prepared(1_010, 100_000, 1_011, 99_000, 0));
    }

    #[test_all]
    fn a_fee_on_a_larger_amount_may_grow_in_proportion() {
        // 1,010 on 100,000 scales to 1,020.1 on 101,000, rounded up.
        assert!(fee_within_prepared(1_010, 100_000, 1_020, 101_000, 0));
        assert!(fee_within_prepared(1_010, 100_000, 1_021, 101_000, 0));
        assert!(!fee_within_prepared(1_010, 100_000, 1_022, 101_000, 0));
    }

    #[test_all]
    fn rounding_allows_a_sat_per_fee() {
        assert!(!fee_within_prepared(100, 10_000, 103, 10_000, 2));
        assert!(fee_within_prepared(100, 10_000, 102, 10_000, 2));
    }

    #[test_all]
    fn a_fee_prepared_on_nothing_stays_capped() {
        assert!(fee_within_prepared(5, 0, 5, 1_000, 0));
        assert!(!fee_within_prepared(5, 0, 6, 1_000, 0));
    }

    #[test_all]
    fn the_slippage_margin_rounds_up() {
        assert_eq!(slippage_margin_sats(100_000, 10), 100);
        assert_eq!(slippage_margin_sats(100_001, 10), 101);
        assert_eq!(slippage_margin_sats(100_000, 0), 0);
        assert_eq!(slippage_margin_sats(u128::MAX, 10_000), u64::MAX);
    }
}
