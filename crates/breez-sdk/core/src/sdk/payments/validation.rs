//! Shared validation building blocks for the send-payment prepare flow.
//!
//! These are the cross-cutting checks that every payment type enforces. Each
//! per-type `prepare/<type>.rs::validate_request` calls them first, so the
//! complete set of rules for an input type is visible in that type's own file.

use breez_sdk_common::input;
use platform_utils::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    ConversionOptions, ConversionType, CrossChainRouteFilter, CrossChainRoutePair, FeePolicy,
    SparkInvoiceDetails,
    cross_chain::{
        DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS, DEFAULT_TARGET_OVERPAY_BPS, MAX_CROSS_CHAIN_SLIPPAGE_BPS,
        MAX_TARGET_OVERPAY_BPS, MIN_CROSS_CHAIN_SLIPPAGE_BPS, MIN_TARGET_OVERPAY_BPS,
    },
    error::SdkError,
    sdk::BreezSdk,
};

/// Validates that a Spark invoice can be paid by us: it has not expired, and it
/// is not bound to a different sender.
pub(in crate::sdk) fn validate_spark_invoice_payable(
    details: &SparkInvoiceDetails,
    identity_public_key: &str,
) -> Result<(), SdkError> {
    if let Some(expiry_time) = details.expiry_time {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SdkError::Generic("Failed to get current time".to_string()))?;
        if current_time > Duration::from_secs(expiry_time) {
            return Err(SdkError::InvalidInput("Invoice has expired".to_string()));
        }
    }

    if let Some(sender_public_key) = &details.sender_public_key
        && identity_public_key != sender_public_key
    {
        return Err(SdkError::InvalidInput(format!(
            "Invoice can only be paid by sender public key {sender_public_key}"
        )));
    }

    Ok(())
}

/// Validates that amount is > 0 if provided.
pub(in crate::sdk) fn validate_amount(amount: Option<u128>) -> Result<(), SdkError> {
    if let Some(0) = amount {
        return Err(SdkError::InvalidInput(
            "Amount must be greater than 0".to_string(),
        ));
    }
    Ok(())
}

/// Validates that `FeesIncluded` is not combined with a `FromBitcoin` conversion.
/// `FeesIncluded` + `ToBitcoin` is allowed (send-all-with-conversion from stable balance).
pub(in crate::sdk) fn validate_fee_policy_for_conversion(
    fee_policy: Option<FeePolicy>,
    conversion_options: Option<&ConversionOptions>,
) -> Result<(), SdkError> {
    if fee_policy == Some(FeePolicy::FeesIncluded)
        && conversion_options.is_some()
        && !matches!(
            conversion_options,
            Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin { .. },
                ..
            })
        )
    {
        return Err(SdkError::InvalidInput(
            "FeesIncluded cannot be combined with FromBitcoin conversion".to_string(),
        ));
    }
    Ok(())
}

/// Human-readable label for a [`input::CrossChainAddressFamily`] suitable
/// for user-facing error messages.
fn address_family_label(family: input::CrossChainAddressFamily) -> &'static str {
    match family {
        input::CrossChainAddressFamily::Evm => "EVM",
        input::CrossChainAddressFamily::Solana => "Solana",
        input::CrossChainAddressFamily::Tron => "Tron",
    }
}

/// Fails when the parsed address's family doesn't match the route's
/// inferable destination family. Skipped when the route is for a
/// native-asset destination (no contract address), so the family can't be
/// inferred from the route alone — the provider validates at submit time.
pub(in crate::sdk) fn validate_address_family_against_route(
    address_family: input::CrossChainAddressFamily,
    route: &CrossChainRoutePair,
) -> Result<(), SdkError> {
    if let Some(route_family) = route.destination_address_family()
        && route_family != address_family
    {
        return Err(SdkError::InvalidInput(format!(
            "Address family ({}) does not match the selected route's chain ({}).",
            address_family_label(address_family),
            route.chain,
        )));
    }
    Ok(())
}

/// Rejects a recipient that is a token contract.
///
/// A TRC20/ERC20 `transfer` to a token's own contract credits the contract's
/// balance, and these contracts expose no way to spend it, so the funds are
/// unrecoverable. Providers screen for this server-side but coverage varies
/// by chain, and by then the Spark leg has already settled.
///
/// `known_contracts` is best-effort: it holds the contracts the route listing
/// surfaced, so an unlisted one still reaches the provider.
pub(in crate::sdk) fn validate_recipient_not_contract_address(
    address: &str,
    address_family: input::CrossChainAddressFamily,
    known_contracts: &[(String, String, String)],
) -> Result<(), SdkError> {
    let matched = known_contracts
        .iter()
        .find(|(contract, ..)| address_family.addresses_equal(contract, address));

    if let Some((_, asset, chain)) = matched {
        return Err(SdkError::InvalidInput(format!(
            "Recipient address is the {asset} token contract on {chain}. \
             Funds sent to a token contract cannot be recovered. \
             Use the wallet address of the recipient instead."
        )));
    }
    Ok(())
}

/// Token contracts to screen the recipient against, as
/// `(contract_address, asset, chain)`, drawn from the route listing plus the
/// selected route itself.
///
/// Both providers scope a `Send` listing to the recipient's address family,
/// so every contract here is comparable to the recipient. Contracts on chains
/// other than the selected route's stay in: pasting another chain's contract
/// is just as unrecoverable, and only an address whose private key is known
/// can be a legitimate recipient, so no wallet address can collide with one.
///
/// A listing that can't be fetched yields whatever the selected route
/// contributes rather than an error: this feeds a best-effort guard, and a
/// provider being briefly unreachable must not block a valid send.
pub(in crate::sdk) async fn known_token_contracts(
    sdk: &BreezSdk,
    address: &str,
    address_family: input::CrossChainAddressFamily,
    route: &CrossChainRoutePair,
) -> Vec<(String, String, String)> {
    let filter = CrossChainRouteFilter::Send {
        address_details: crate::common::models::CrossChainAddressDetails {
            address: address.to_string(),
            address_family: address_family.into(),
            // Left unset so the listing isn't narrowed to a single asset: a
            // paste of some *other* token's contract has to match too.
            contract_address: None,
            chain_id: None,
            amount: None,
        },
    };

    let mut listed = match sdk.get_cross_chain_routes(&filter).await {
        Ok(routes) => routes,
        Err(e) => {
            tracing::warn!(
                "Cross-chain: contract-address guard falling back to the selected route: {e}"
            );
            Vec::new()
        }
    };
    listed.push(route.clone());

    listed
        .into_iter()
        .filter_map(|r| r.contract_address.map(|c| (c, r.asset, r.chain)))
        .collect()
}

/// Resolves the slippage bps to use for the provider leg.
///
/// Caller-supplied values must lie in `MIN_…..=MAX_…`. Otherwise falls back
/// to the config default and finally to the built-in default.
/// Config defaults are validated at SDK startup in `Config::validate`, so
/// only the request-supplied value is bound-checked here.
pub(in crate::sdk) fn resolve_slippage_bps(
    requested: Option<u32>,
    config_default: Option<u32>,
) -> Result<u32, SdkError> {
    if let Some(bps) = requested
        && !(MIN_CROSS_CHAIN_SLIPPAGE_BPS..=MAX_CROSS_CHAIN_SLIPPAGE_BPS).contains(&bps)
    {
        return Err(SdkError::InvalidInput(format!(
            "max_slippage_bps {bps} must be in \
             {MIN_CROSS_CHAIN_SLIPPAGE_BPS}..={MAX_CROSS_CHAIN_SLIPPAGE_BPS}",
        )));
    }
    Ok(requested
        .or(config_default)
        .unwrap_or(DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS))
}

/// Resolves the target-overpay bps to apply on `FeesExcluded` orders.
///
/// Same precedence as slippage: caller-supplied value (bounds-checked here),
/// then the config default, then the built-in default. Config defaults are
/// validated at SDK startup in `Config::validate`.
pub(in crate::sdk) fn resolve_target_overpay_bps(
    requested: Option<u32>,
    config_default: Option<u32>,
) -> Result<u32, SdkError> {
    if let Some(bps) = requested
        && !(MIN_TARGET_OVERPAY_BPS..=MAX_TARGET_OVERPAY_BPS).contains(&bps)
    {
        return Err(SdkError::InvalidInput(format!(
            "target_overpay_bps {bps} must be in \
             {MIN_TARGET_OVERPAY_BPS}..={MAX_TARGET_OVERPAY_BPS}",
        )));
    }
    Ok(requested
        .or(config_default)
        .unwrap_or(DEFAULT_TARGET_OVERPAY_BPS))
}

/// Inflates a destination amount by `overpay_bps` so the recipient lands at
/// or above target despite provider slippage. `overpay_bps == 0` is identity.
pub(in crate::sdk) fn inflate_target_amount(amount: u128, overpay_bps: u32) -> u128 {
    if overpay_bps == 0 {
        return amount;
    }
    amount.saturating_add(amount.saturating_mul(u128::from(overpay_bps)) / 10_000)
}

/// Pads a sats-denominated order for `FeesExcluded` so the recipient lands at
/// or above target despite provider slippage (`FeesIncluded` fixes the payer's
/// spend, so it passes through). Padding sats to hit a USD target is valid
/// because the route aggregator lists only USD-pegged destinations: parity
/// holds for both a BTC source (sats fiat-inverted by the provider) and a
/// stable-token source (parity rescale).
pub(in crate::sdk) fn resolve_direct_overpay_amount(
    amount: u128,
    fee_policy: FeePolicy,
    overpay_bps: u32,
) -> u128 {
    if overpay_bps == 0 || !matches!(fee_policy, FeePolicy::FeesExcluded) {
        return amount;
    }
    inflate_target_amount(amount, overpay_bps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CrossChainProvider, SourceAsset, SourceChain};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn from_bitcoin_options() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        }
    }

    fn to_bitcoin_options() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "token123".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        }
    }

    #[test_all]
    fn test_validate_amount_zero() {
        let result = validate_amount(Some(0));
        assert!(result.is_err(), "Should fail for zero amount");
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("must be greater than 0"),
                "Error message should mention requirement"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_amount_none_and_positive_ok() {
        assert!(validate_amount(None).is_ok());
        assert!(validate_amount(Some(1)).is_ok());
    }

    #[test_all]
    fn test_validate_fees_included_with_from_bitcoin_conversion_fails() {
        let result = validate_fee_policy_for_conversion(
            Some(FeePolicy::FeesIncluded),
            Some(&from_bitcoin_options()),
        );
        assert!(
            result.is_err(),
            "Should fail for FeesIncluded with FromBitcoin conversion"
        );
        if let Err(SdkError::InvalidInput(msg)) = result {
            assert!(
                msg.contains("FeesIncluded cannot be combined with FromBitcoin conversion"),
                "Error message should mention FeesIncluded and FromBitcoin conversion"
            );
        } else {
            panic!("Expected InvalidInput error");
        }
    }

    #[test_all]
    fn test_validate_fees_included_with_to_bitcoin_conversion_succeeds() {
        assert!(
            validate_fee_policy_for_conversion(
                Some(FeePolicy::FeesIncluded),
                Some(&to_bitcoin_options())
            )
            .is_ok(),
            "Should succeed for FeesIncluded with ToBitcoin conversion (send-all-with-conversion)"
        );
    }

    #[test_all]
    fn test_validate_fee_policy_no_conversion_ok() {
        // FeesIncluded with no conversion options is fine.
        assert!(validate_fee_policy_for_conversion(Some(FeePolicy::FeesIncluded), None).is_ok());
    }

    // ---- resolve_slippage_bps ----

    #[test_all]
    fn resolve_slippage_uses_request_when_in_range() {
        assert_eq!(resolve_slippage_bps(Some(50), Some(200)).unwrap(), 50);
    }

    #[test_all]
    fn resolve_slippage_falls_back_to_config_when_request_none() {
        assert_eq!(resolve_slippage_bps(None, Some(200)).unwrap(), 200);
    }

    #[test_all]
    fn resolve_slippage_falls_back_to_built_in_when_both_none() {
        assert_eq!(
            resolve_slippage_bps(None, None).unwrap(),
            DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS
        );
    }

    #[test_all]
    fn resolve_slippage_rejects_below_min() {
        let too_low = MIN_CROSS_CHAIN_SLIPPAGE_BPS - 1;
        let err = resolve_slippage_bps(Some(too_low), None).unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(
            msg.contains(&too_low.to_string()),
            "error message should mention the bad bps value (got: {msg})"
        );
    }

    #[test_all]
    fn resolve_slippage_rejects_above_max() {
        let too_high = MAX_CROSS_CHAIN_SLIPPAGE_BPS + 1;
        assert!(matches!(
            resolve_slippage_bps(Some(too_high), None),
            Err(SdkError::InvalidInput(_))
        ));
    }

    // ---- resolve_target_overpay_bps ----

    #[test_all]
    fn resolve_overpay_uses_request_when_in_range() {
        assert_eq!(resolve_target_overpay_bps(Some(50), Some(75)).unwrap(), 50);
    }

    #[test_all]
    fn resolve_overpay_falls_back_to_config_when_request_none() {
        assert_eq!(resolve_target_overpay_bps(None, Some(75)).unwrap(), 75);
    }

    #[test_all]
    fn resolve_overpay_falls_back_to_built_in_when_both_none() {
        assert_eq!(
            resolve_target_overpay_bps(None, None).unwrap(),
            DEFAULT_TARGET_OVERPAY_BPS
        );
    }

    #[test_all]
    fn resolve_overpay_accepts_zero_to_opt_out() {
        assert_eq!(resolve_target_overpay_bps(Some(0), Some(50)).unwrap(), 0);
    }

    #[test_all]
    fn resolve_overpay_rejects_above_max() {
        let too_high = MAX_TARGET_OVERPAY_BPS + 1;
        assert!(matches!(
            resolve_target_overpay_bps(Some(too_high), None),
            Err(SdkError::InvalidInput(_))
        ));
    }

    // ---- inflate_target_amount ----

    #[test_all]
    fn inflate_target_amount_zero_bps_is_identity() {
        assert_eq!(inflate_target_amount(1_000_000, 0), 1_000_000);
    }

    #[test_all]
    fn inflate_target_amount_applies_bps_pad() {
        // 25 bps on 1_000_000 -> 1_000_000 + 2_500 = 1_002_500.
        assert_eq!(inflate_target_amount(1_000_000, 25), 1_002_500);
    }

    #[test_all]
    fn inflate_target_amount_truncates_sub_unit_pad() {
        // 25 bps on 100 -> 100 + (100 * 25 / 10_000) = 100 + 0 (integer floor).
        // Acceptable: pad is sub-unit for tiny amounts.
        assert_eq!(inflate_target_amount(100, 25), 100);
    }

    #[test_all]
    fn resolve_direct_overpay_pads_only_fees_excluded() {
        // FeesExcluded pads; FeesIncluded and zero-bps are identity.
        assert_eq!(
            resolve_direct_overpay_amount(1_000_000, FeePolicy::FeesExcluded, 25),
            1_002_500
        );
        assert_eq!(
            resolve_direct_overpay_amount(1_000_000, FeePolicy::FeesIncluded, 25),
            1_000_000
        );
        assert_eq!(
            resolve_direct_overpay_amount(1_000_000, FeePolicy::FeesExcluded, 0),
            1_000_000
        );
    }

    // ---- validate_address_family_against_route ----

    fn route_with_contract(chain: &str, contract: Option<&str>) -> CrossChainRoutePair {
        CrossChainRoutePair {
            provider: CrossChainProvider::Orchestra,
            chain: chain.to_string(),
            chain_id: None,
            asset: "USDC".to_string(),
            contract_address: contract.map(str::to_string),
            decimals: 6,
            exact_out_eligible: false,
            supported_sources: vec![SourceAsset::Bitcoin],
            supported_source_chains: vec![SourceChain::Spark],
        }
    }

    #[test_all]
    fn destination_family_inferred_from_evm_contract() {
        let route = route_with_contract("base", Some("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"));
        assert_eq!(
            route.destination_address_family(),
            Some(input::CrossChainAddressFamily::Evm)
        );
    }

    #[test_all]
    fn destination_family_none_for_native_route() {
        let route = route_with_contract("base", None);
        assert!(route.destination_address_family().is_none());
    }

    #[test_all]
    fn family_check_passes_when_route_matches_address() {
        let route = route_with_contract("base", Some("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"));
        assert!(
            validate_address_family_against_route(input::CrossChainAddressFamily::Evm, &route)
                .is_ok()
        );
    }

    #[test_all]
    fn family_check_rejects_mismatched_route() {
        let route = route_with_contract("base", Some("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"));
        let err =
            validate_address_family_against_route(input::CrossChainAddressFamily::Solana, &route)
                .unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("Solana") && msg.contains("base"));
    }

    #[test_all]
    fn family_check_skips_when_route_has_no_contract() {
        let route = route_with_contract("base", None);
        // No contract → family can't be inferred from the route, so the check
        // is a no-op and we defer to the provider.
        assert!(
            validate_address_family_against_route(input::CrossChainAddressFamily::Solana, &route)
                .is_ok()
        );
    }

    // ---- validate_recipient_not_contract_address ----

    const USDT_TRON: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
    const USDC_ARBITRUM: &str = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831";

    const USDC_BASE: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";

    fn contracts(triples: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        triples
            .iter()
            .map(|(c, a, ch)| ((*c).to_string(), (*a).to_string(), (*ch).to_string()))
            .collect()
    }

    #[test_all]
    fn contract_check_rejects_the_routes_own_contract() {
        let err = validate_recipient_not_contract_address(
            USDT_TRON,
            input::CrossChainAddressFamily::Tron,
            &contracts(&[(USDT_TRON, "USDT", "tron")]),
        )
        .unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("USDT") && msg.contains("tron"));
    }

    #[test_all]
    fn contract_check_rejects_another_assets_contract_on_the_same_chain() {
        // Recipient is the USDT contract while the selected route is USDC, so
        // comparing against the route alone would let this through.
        let err = validate_recipient_not_contract_address(
            USDT_TRON,
            input::CrossChainAddressFamily::Tron,
            &contracts(&[
                ("TEkxiTehnzSmSe2XqrBj4w32RUN966rdz8", "USDC", "tron"),
                (USDT_TRON, "USDT", "tron"),
            ]),
        )
        .unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("USDT"));
    }

    #[test_all]
    fn contract_check_rejects_a_contract_from_another_chain() {
        // Selected route is Base, recipient is Arbitrum's USDC contract. Just
        // as unrecoverable, and the message names the chain the contract is on.
        let err = validate_recipient_not_contract_address(
            USDC_ARBITRUM,
            input::CrossChainAddressFamily::Evm,
            &contracts(&[
                (USDC_BASE, "USDC", "base"),
                (USDC_ARBITRUM, "USDC", "arbitrum"),
            ]),
        )
        .unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("arbitrum"), "{msg}");
    }

    #[test_all]
    fn contract_check_is_case_insensitive_for_evm() {
        assert!(
            validate_recipient_not_contract_address(
                &USDC_ARBITRUM.to_lowercase(),
                input::CrossChainAddressFamily::Evm,
                &contracts(&[(USDC_ARBITRUM, "USDC", "arbitrum")]),
            )
            .is_err()
        );
    }

    #[test_all]
    fn contract_check_is_case_sensitive_for_base58() {
        // Base58 case carries information, so a case-folded string is a
        // different address, not the contract.
        assert!(
            validate_recipient_not_contract_address(
                &USDT_TRON.to_lowercase(),
                input::CrossChainAddressFamily::Tron,
                &contracts(&[(USDT_TRON, "USDT", "tron")]),
            )
            .is_ok()
        );
    }

    #[test_all]
    fn contract_check_allows_a_wallet_address() {
        assert!(
            validate_recipient_not_contract_address(
                "0x9d9089eE0D37EF0815336de64FbBCB8a99da8344",
                input::CrossChainAddressFamily::Evm,
                &contracts(&[(USDC_ARBITRUM, "USDC", "arbitrum")]),
            )
            .is_ok()
        );
    }

    #[test_all]
    fn contract_check_is_a_no_op_without_known_contracts() {
        assert!(
            validate_recipient_not_contract_address(
                USDT_TRON,
                input::CrossChainAddressFamily::Tron,
                &[],
            )
            .is_ok()
        );
    }
}
