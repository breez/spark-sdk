use bitcoin::{ScriptBuf, WitnessProgram, WitnessVersion, address::NetworkUnchecked};
use spark_wallet::SilentPaymentAddress;

use crate::error::SdkError;

/// Returns the minimum non-dust amount in sats for the given Bitcoin address
/// or silent payment address.
pub(crate) fn get_dust_limit_sats(address: &str) -> Result<u64, SdkError> {
    let addr = match address.parse::<bitcoin::Address<NetworkUnchecked>>() {
        Ok(addr) => addr,
        Err(_) if address.parse::<SilentPaymentAddress>().is_ok() => {
            return Ok(taproot_dust_limit_sats());
        }
        Err(e) => return Err(SdkError::InvalidInput(format!("Invalid address: {e}"))),
    };
    Ok(addr
        .assume_checked()
        .script_pubkey()
        .minimal_non_dust()
        .to_sat())
}

/// A payment to a silent payment address is a taproot output. Every taproot
/// output script has the same length, so they share one dust limit.
fn taproot_dust_limit_sats() -> u64 {
    let program = WitnessProgram::new(WitnessVersion::V1, &[0u8; 32])
        .expect("32 bytes make a taproot witness program");
    ScriptBuf::new_witness_program(&program)
        .minimal_non_dust()
        .to_sat()
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
    fn test_dust_limit_silent_payment_address() {
        let result = get_dust_limit_sats(
            "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
        );
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
