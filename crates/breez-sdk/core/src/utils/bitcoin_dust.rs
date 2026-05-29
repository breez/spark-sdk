use bitcoin::address::NetworkUnchecked;

use crate::error::SdkError;

/// Returns the minimum non-dust amount in sats for the given Bitcoin address.
pub(crate) fn get_dust_limit_sats(address: &str) -> Result<u64, SdkError> {
    let addr = address
        .parse::<bitcoin::Address<NetworkUnchecked>>()
        .map_err(|e| SdkError::InvalidInput(format!("Invalid address: {e}")))?;
    Ok(addr
        .assume_checked()
        .script_pubkey()
        .minimal_non_dust()
        .to_sat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_dust_limit_p2tr() {
        let result =
            get_dust_limit_sats("bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297");
        assert_eq!(result.unwrap(), 330);
    }

    #[test_all]
    fn test_dust_limit_p2wpkh() {
        let result = get_dust_limit_sats("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4");
        assert_eq!(result.unwrap(), 294);
    }

    #[test_all]
    fn test_dust_limit_p2pkh() {
        let result = get_dust_limit_sats("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa");
        assert_eq!(result.unwrap(), 546);
    }

    #[test_all]
    fn test_dust_limit_invalid_address() {
        let result = get_dust_limit_sats("not_an_address");
        assert!(result.is_err());
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(msg.contains("Invalid address"));
        } else {
            panic!("Expected InvalidInput error");
        }
    }
}
