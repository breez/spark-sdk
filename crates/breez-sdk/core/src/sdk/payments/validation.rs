//! Shared validation building blocks for the send-payment prepare flow, and
//! for the refund address a cross-chain receive or bridge takes.
//!
//! These are the cross-cutting checks that every payment type enforces. Each
//! per-type `prepare/<type>.rs::validate_request` calls them first, so the
//! complete set of rules for an input type is visible in that type's own file.

use breez_sdk_common::input;

use crate::sdk::SdkInputParser;
use crate::utils::time::try_now_secs;
use crate::{
    Bolt11InvoiceDetails, ConversionOptions, ConversionType, CrossChainRouteFilter,
    CrossChainRoutePair, FeePolicy, InputType, SparkInvoiceDetails,
    cross_chain::{
        CrossChainContext, DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS, MAX_CROSS_CHAIN_SLIPPAGE_BPS,
        MIN_CROSS_CHAIN_SLIPPAGE_BPS,
    },
    error::SdkError,
};

/// Validates that a Spark invoice can be paid by us: it has not expired, and it
/// is not bound to a different sender.
pub(in crate::sdk) fn validate_spark_invoice_payable(
    details: &SparkInvoiceDetails,
    identity_public_key: &str,
) -> Result<(), SdkError> {
    if let Some(expiry_time) = details.expiry_time
        && try_now_secs()? > expiry_time
    {
        return Err(SdkError::InvalidInput("Invoice has expired".to_string()));
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

/// Validates that a Bolt11 invoice has not expired.
pub(in crate::sdk) fn validate_bolt11_invoice_not_expired(
    details: &Bolt11InvoiceDetails,
) -> Result<(), SdkError> {
    if try_now_secs()? > details.timestamp.saturating_add(details.expiry) {
        return Err(SdkError::InvalidInput("Invoice has expired".to_string()));
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
    cross_chain_context: &CrossChainContext,
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
        delivery_method: None,
    };

    let mut listed = match cross_chain_context.list_routes(&filter).await {
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

/// Parses the payer's refund address, when given, and checks it is an address
/// on `route`'s chain that isn't a token contract.
pub(in crate::sdk) async fn validate_refund_address(
    cross_chain_context: &CrossChainContext,
    input_parser: &SdkInputParser,
    refund_address: Option<&str>,
    route: &CrossChainRoutePair,
) -> Result<Option<String>, SdkError> {
    let Some(refund_address) = refund_address else {
        return Ok(None);
    };
    let parsed: InputType = input_parser.parse(refund_address).await?.into();
    let InputType::CrossChainAddress(refund) = parsed else {
        return Err(SdkError::InvalidInput(
            "Refund address must be a cross-chain (EVM/Solana/Tron) address".to_string(),
        ));
    };
    let refund_family: input::CrossChainAddressFamily = refund.address_family.into();
    validate_address_family_against_route(refund_family, route)?;
    let known_contracts =
        known_token_contracts(cross_chain_context, &refund.address, refund_family, route).await;
    validate_recipient_not_contract_address(&refund.address, refund_family, &known_contracts)
        .map_err(|_| {
            SdkError::InvalidInput(
                "Refund address is a token contract. Funds refunded to it cannot be \
             recovered. Use the payer's wallet address instead."
                    .to_string(),
            )
        })?;
    Ok(Some(refund.address))
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
             {MIN_CROSS_CHAIN_SLIPPAGE_BPS} to {MAX_CROSS_CHAIN_SLIPPAGE_BPS}",
        )));
    }
    Ok(requested
        .or(config_default)
        .unwrap_or(DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS))
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
    crate::cross_chain::inflate_target_amount(amount, overpay_bps)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{CrossChainAcceptedAsset, CrossChainProvider, DeliveryMethod, SparkAsset};
    use macros::{async_test_all, test_all};

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
            accepted_assets: vec![CrossChainAcceptedAsset {
                asset: SparkAsset::Bitcoin,
                limits: None,
            }],
            delivery_methods: vec![DeliveryMethod::Spark],
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

    // ---- validate_refund_address ----

    const PAYER_TRON: &str = "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9";
    const PAYER_EVM: &str = "0x9d9089eE0D37EF0815336de64FbBCB8a99da8344";

    struct NoFiat;

    #[macros::async_trait]
    impl breez_sdk_common::fiat::FiatService for NoFiat {
        async fn fetch_fiat_currencies(
            &self,
        ) -> Result<
            Vec<breez_sdk_common::fiat::FiatCurrency>,
            breez_sdk_common::error::ServiceConnectivityError,
        > {
            panic!("unexpected fiat lookup")
        }

        async fn fetch_fiat_rates(
            &self,
        ) -> Result<
            Vec<breez_sdk_common::fiat::Rate>,
            breez_sdk_common::error::ServiceConnectivityError,
        > {
            panic!("unexpected fiat lookup")
        }
    }

    struct Offline;

    #[macros::async_trait]
    impl platform_utils::HttpClient for Offline {
        async fn get(
            &self,
            url: String,
            _: Option<std::collections::HashMap<String, String>>,
        ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
            panic!("unexpected GET {url}")
        }
        async fn post(
            &self,
            url: String,
            _: Option<std::collections::HashMap<String, String>>,
            _: Option<String>,
        ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
            panic!("unexpected POST {url}")
        }
        async fn delete(
            &self,
            url: String,
            _: Option<std::collections::HashMap<String, String>>,
            _: Option<String>,
        ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
            panic!("unexpected DELETE {url}")
        }
    }

    /// Checks `refund_address` against a Tron USDT route, offline. With no
    /// provider to list routes, the token contracts known are the route's own.
    async fn check_tron_refund(refund_address: Option<&str>) -> Result<Option<String>, SdkError> {
        let context = CrossChainContext::new(Arc::new(NoFiat));
        let parser = SdkInputParser::new(
            breez_sdk_common::dns::Resolver::with_proxy(None).unwrap(),
            Arc::new(Offline),
            None,
        );
        let route = route_with_contract("tron", Some(USDT_TRON));
        validate_refund_address(&context, &parser, refund_address, &route).await
    }

    fn invalid_input_message(result: Result<Option<String>, SdkError>) -> String {
        match result {
            Err(SdkError::InvalidInput(msg)) => msg,
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[async_test_all]
    async fn a_refund_address_is_optional() {
        assert_eq!(check_tron_refund(None).await.unwrap(), None);
    }

    #[async_test_all]
    async fn a_refund_address_on_the_routes_chain_is_accepted() {
        assert_eq!(
            check_tron_refund(Some(PAYER_TRON))
                .await
                .unwrap()
                .as_deref(),
            Some(PAYER_TRON)
        );
    }

    #[async_test_all]
    async fn a_refund_address_on_another_chain_is_rejected() {
        let msg = invalid_input_message(check_tron_refund(Some(PAYER_EVM)).await);
        assert!(msg.contains("tron"), "{msg}");
    }

    #[async_test_all]
    async fn a_token_contract_is_not_a_refund_address() {
        let msg = invalid_input_message(check_tron_refund(Some(USDT_TRON)).await);
        assert!(msg.contains("token contract"), "{msg}");
    }

    #[async_test_all]
    async fn a_refund_address_must_be_a_cross_chain_address() {
        let spark_address = "spark1pgss98a8ctz6t632ss24c4xst2axg9r9lac38lvhq7v79t4x8sdcme3jcqeruk";
        let msg = invalid_input_message(check_tron_refund(Some(spark_address)).await);
        assert!(msg.contains("cross-chain"), "{msg}");
    }
}
