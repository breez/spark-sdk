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

#[cfg(test)]
mod tests {
    use super::fee_overpayment;
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
}
