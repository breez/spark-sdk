use breez_sdk_common::input;

use crate::{
    ConversionEstimate, ConversionOptions, ConversionType, CrossChainFeeMode, CrossChainRoutePair,
    FeePolicy, SendPaymentMethod,
    cross_chain::{
        DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS, MAX_CROSS_CHAIN_SLIPPAGE_BPS,
        MIN_CROSS_CHAIN_SLIPPAGE_BPS, SourceAsset,
    },
    error::SdkError,
    models::PrepareSendPaymentResponse,
    sdk::BreezSdk,
};

use super::super::{conversion, validation};

/// Pre-resolution validation for cross-chain sends. Shared checks
/// (`validate_amount`) plus the cross-chain-specific rule that
/// `FromBitcoin` conversions are unsupported.
fn validate_request(
    amount: u128,
    conversion_options: Option<&ConversionOptions>,
) -> Result<(), SdkError> {
    validation::validate_amount(Some(amount))?;

    if let Some(ConversionOptions {
        conversion_type: ConversionType::FromBitcoin,
        ..
    }) = conversion_options
    {
        return Err(SdkError::InvalidInput(
            "FromBitcoin conversion is not supported for cross-chain sends.".to_string(),
        ));
    }

    Ok(())
}

/// Post-resolution check: the effective source asset (the one the wallet
/// actually pays on, after the source-selection decision tree) must be in
/// the route's `supported_sources`.
fn validate_route_supports_effective_source(
    route: &CrossChainRoutePair,
    token_identifier: Option<&String>,
    effective_conversion_options: Option<&ConversionOptions>,
) -> Result<(), SdkError> {
    let effective_source = if effective_conversion_options.is_some() {
        // Conversion runs before the provider leg → source is always sats.
        SourceAsset::Bitcoin
    } else {
        match token_identifier {
            Some(tid) => SourceAsset::Token {
                token_identifier: tid.clone(),
            },
            None => SourceAsset::Bitcoin,
        }
    };

    if !route.supported_sources.contains(&effective_source) {
        let supported_list = route
            .supported_sources
            .iter()
            .map(|s| match s {
                SourceAsset::Bitcoin => "sats".to_string(),
                SourceAsset::Token {
                    token_identifier: t,
                } => t.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SdkError::InvalidInput(match token_identifier {
            Some(tid) => format!(
                "Route does not accept source asset {tid}. Supported: {supported_list}. Provide a token_identifier matching one of the supported sources, or pick another route."
            ),
            None => format!(
                "Route does not accept sats. Provide a token_identifier matching one of: {supported_list}."
            ),
        }));
    }

    Ok(())
}

/// Resolves the slippage bps to use for the provider leg.
///
/// Caller-supplied values must lie in `MIN_…..=MAX_…`. Otherwise falls back
/// to the config default and finally to the built-in default.
/// Config defaults are validated at SDK startup in `Config::validate`, so
/// only the request-supplied value is bound-checked here.
fn resolve_slippage_bps(
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

/// Decides whether the source path requires the converted-sats budget to be
/// treated as `FeesIncluded`.
///
/// Conversion sends have no coherent partition for `FeesExcluded`: the
/// wallet's only sats budget comes from the AMM output, so fees can only
/// come out of that budget. Force `FeesIncluded` regardless of what the
/// caller passed — keeps the stable-balance illusion intact for callers
/// that default to `FeesExcluded` for sat sends.
fn effective_fee_policy(is_conversion: bool, requested: FeePolicy) -> FeePolicy {
    if is_conversion {
        FeePolicy::FeesIncluded
    } else {
        requested
    }
}

/// Runs the token-conversion estimate for the source leg and validates the
/// caller's token balance covers the conversion input.
///
/// Returns the estimated sats output (used as the provider-leg amount) and
/// the `ConversionEstimate` to attach to the prepare response.
async fn estimate_and_validate_conversion(
    sdk: &BreezSdk,
    opts: &ConversionOptions,
    token_identifier: Option<&String>,
    amount: u128,
    fee_policy: FeePolicy,
) -> Result<(u128, Option<ConversionEstimate>), SdkError> {
    let (estimated_sats, estimate) = conversion::estimate_sats_from_token_conversion(
        sdk,
        opts,
        token_identifier,
        amount,
        fee_policy,
    )
    .await?;

    if let Some(ref ce) = estimate
        && let ConversionType::ToBitcoin {
            from_token_identifier,
        } = &ce.options.conversion_type
    {
        let balances = sdk.spark_wallet.get_token_balances().await?;
        let have = balances
            .get(from_token_identifier)
            .map_or(0u128, |b| b.balance);
        if have < ce.amount_in {
            return Err(SdkError::InvalidInput(format!(
                "Insufficient {from_token_identifier} balance for conversion: have {have}, need {}.",
                ce.amount_in
            )));
        }
    }

    Ok((estimated_sats, estimate))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare(
    sdk: &BreezSdk,
    address: &str,
    route: &CrossChainRoutePair,
    amount: u128,
    token_identifier: Option<String>,
    conversion_options: Option<ConversionOptions>,
    fee_policy: FeePolicy,
    max_slippage_bps: Option<u32>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    validate_request(amount, conversion_options.as_ref())?;

    if input::detect_address_family(address).is_none() {
        return Err(SdkError::InvalidInput(
            "Address is not a recognized cross-chain address".to_string(),
        ));
    }

    let provider_slippage_bps = resolve_slippage_bps(
        max_slippage_bps,
        sdk.config
            .cross_chain_config
            .as_ref()
            .and_then(|c| c.default_slippage_bps),
    )?;

    // Source-selection decision tree → effective conversion options.
    let effective_conversion_options = resolve_cross_chain_source(
        sdk,
        route,
        token_identifier.as_ref(),
        conversion_options.as_ref(),
        amount,
    )
    .await?;
    validate_route_supports_effective_source(
        route,
        token_identifier.as_ref(),
        effective_conversion_options.as_ref(),
    )?;

    let effective_fee_policy =
        effective_fee_policy(effective_conversion_options.is_some(), fee_policy);

    let fee_mode = match effective_fee_policy {
        FeePolicy::FeesExcluded => CrossChainFeeMode::FeesExcluded,
        FeePolicy::FeesIncluded => CrossChainFeeMode::FeesIncluded,
    };

    // Provider-leg amount + conversion-estimate metadata.
    //
    // `source_token_identifier` is `None` on the conversion path because the
    // converted output is sats — both the provider leg and the response use
    // it directly without a token denomination.
    let (provider_amount, conversion_estimate, source_token_identifier) =
        match effective_conversion_options.as_ref() {
            Some(opts) => {
                let (sats, estimate) = estimate_and_validate_conversion(
                    sdk,
                    opts,
                    token_identifier.as_ref(),
                    amount,
                    effective_fee_policy,
                )
                .await?;
                (sats, estimate, None)
            }
            None => (amount, None, token_identifier.clone()),
        };

    let service = sdk.cross_chain_providers.get(route.provider)?;
    let prepared = service
        .prepare(
            address,
            route,
            provider_amount,
            source_token_identifier.clone(),
            provider_slippage_bps,
            fee_mode,
        )
        .await?;

    Ok(PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::CrossChainAddress {
            route: prepared.pair,
            recipient_address: prepared.recipient_address,
            amount_in: prepared.amount_in,
            estimated_out: prepared.estimated_out,
            fee_amount: prepared.fee_amount,
            fee_asset: prepared.fee_asset,
            source_transfer_fee_sats: prepared.source_transfer_fee_sats,
            fee_mode: prepared.fee_mode,
            expires_at: prepared.expires_at,
            provider_context: prepared.provider_context,
        },
        amount: provider_amount,
        token_identifier: source_token_identifier,
        conversion_estimate,
        fee_policy: effective_fee_policy,
    })
}

/// Resolves the effective `conversion_options` for a cross-chain send.
///
/// Decision tree: explicit caller intent wins; then direct send if the route
/// accepts the user's `token_identifier`; then auto-inject `ToBitcoin` if the
/// token is the stable-balance active token and the route accepts sats; then
/// defer to the stable-balance sats-side auto-inject when no
/// `token_identifier` was supplied — the inner
/// `stable_balance.get_conversion_options` checks `balance_sats >= amount` to
/// decide whether a top-up conversion is needed.
async fn resolve_cross_chain_source(
    sdk: &BreezSdk,
    route: &CrossChainRoutePair,
    token_identifier: Option<&String>,
    conversion_options: Option<&ConversionOptions>,
    amount: u128,
) -> Result<Option<ConversionOptions>, SdkError> {
    let active_stable_token = match &sdk.stable_balance {
        Some(sb) => sb.get_active_token_identifier().await,
        None => None,
    };
    let stable_max_slippage_bps = sdk
        .stable_balance
        .as_ref()
        .and_then(|sb| sb.config.max_slippage_bps);

    match decide_cross_chain_source(
        route,
        token_identifier,
        conversion_options,
        active_stable_token.as_ref(),
        stable_max_slippage_bps,
    ) {
        CrossChainSourceDecision::UseAsIs(opts) => Ok(opts),
        CrossChainSourceDecision::DeferToStableBalance => {
            if let Some(stable_balance) = &sdk.stable_balance {
                stable_balance
                    .get_conversion_options(None, None, amount)
                    .await
                    .map_err(Into::into)
            } else {
                Ok(None)
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum CrossChainSourceDecision {
    UseAsIs(Option<ConversionOptions>),
    DeferToStableBalance,
}

/// Decides the source-side conversion options for a cross-chain send.
///
/// Separated from [`resolve_cross_chain_source`] so the decision tree can be
/// unit-tested without an `SdkContext`. The wrapper feeds in the
/// stable-balance state and acts on the [`DeferToStableBalance`] outcome.
///
/// Inputs:
/// - `active_stable_token` — `Some(id)` when stable-balance is configured AND
///   has an active token; `None` otherwise.
/// - `stable_max_slippage_bps` — slippage from `StableBalanceConfig` to put on
///   the auto-injected `ToBitcoin` options when stable-balance fires.
///
/// [`DeferToStableBalance`]: CrossChainSourceDecision::DeferToStableBalance
fn decide_cross_chain_source(
    route: &CrossChainRoutePair,
    token_identifier: Option<&String>,
    conversion_options: Option<&ConversionOptions>,
    active_stable_token: Option<&String>,
    stable_max_slippage_bps: Option<u32>,
) -> CrossChainSourceDecision {
    // 1) Explicit caller intent wins.
    if let Some(opts) = conversion_options {
        return CrossChainSourceDecision::UseAsIs(Some(opts.clone()));
    }

    match token_identifier {
        // 2) Caller specified a token.
        Some(token_id) => {
            let route_accepts_token = route
                .supported_sources
                .iter()
                .any(|s| matches!(s, SourceAsset::Token { token_identifier: t } if t == token_id));
            if route_accepts_token {
                // Direct token send — no conversion needed.
                return CrossChainSourceDecision::UseAsIs(None);
            }

            let route_accepts_bitcoin = route.supported_sources.contains(&SourceAsset::Bitcoin);
            if !route_accepts_bitcoin {
                // Neither direct nor auto-inject; caller must handle.
                return CrossChainSourceDecision::UseAsIs(None);
            }

            // Auto-inject ToBitcoin if and only if `token_id` is the
            // stable-balance active token.
            if active_stable_token == Some(token_id) {
                return CrossChainSourceDecision::UseAsIs(Some(ConversionOptions {
                    conversion_type: ConversionType::ToBitcoin {
                        from_token_identifier: token_id.clone(),
                    },
                    max_slippage_bps: stable_max_slippage_bps,
                    completion_timeout_secs: None,
                }));
            }
            CrossChainSourceDecision::UseAsIs(None)
        }
        // 3) No token specified.
        None => {
            if route.supported_sources.contains(&SourceAsset::Bitcoin) {
                // Defer to stable_balance auto-inject: fires when the sats
                // balance is insufficient to cover `amount`.
                CrossChainSourceDecision::DeferToStableBalance
            } else {
                // Route only accepts tokens but user didn't specify one.
                // FromBitcoin + CrossChain is out of scope; nothing to
                // auto-inject. Leave as None; post-validation will error.
                CrossChainSourceDecision::UseAsIs(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CrossChainProvider, error::SdkError};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

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

    // ---- effective_fee_policy ----

    #[test_all]
    fn effective_fee_policy_forces_fees_included_on_conversion() {
        assert_eq!(
            effective_fee_policy(true, FeePolicy::FeesExcluded),
            FeePolicy::FeesIncluded
        );
        assert_eq!(
            effective_fee_policy(true, FeePolicy::FeesIncluded),
            FeePolicy::FeesIncluded
        );
    }

    #[test_all]
    fn effective_fee_policy_passes_through_without_conversion() {
        assert_eq!(
            effective_fee_policy(false, FeePolicy::FeesExcluded),
            FeePolicy::FeesExcluded
        );
        assert_eq!(
            effective_fee_policy(false, FeePolicy::FeesIncluded),
            FeePolicy::FeesIncluded
        );
    }

    // ---- validate_request ----

    #[test_all]
    fn validate_request_rejects_zero_amount() {
        let err = validate_request(0, None).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test_all]
    fn validate_request_rejects_from_bitcoin_conversion() {
        let opts = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        };
        let err = validate_request(1000, Some(&opts)).unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("FromBitcoin"));
    }

    #[test_all]
    fn validate_request_allows_to_bitcoin_conversion() {
        let opts = ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "USDB".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        };
        assert!(validate_request(1000, Some(&opts)).is_ok());
    }

    #[test_all]
    fn validate_request_allows_no_conversion() {
        assert!(validate_request(1000, None).is_ok());
    }

    // ---- decide_cross_chain_source ----

    fn route_with_sources(sources: Vec<SourceAsset>) -> CrossChainRoutePair {
        CrossChainRoutePair {
            provider: CrossChainProvider::Orchestra,
            chain: "base".to_string(),
            chain_id: Some("8453".to_string()),
            asset: "USDC".to_string(),
            contract_address: None,
            decimals: 6,
            exact_out_eligible: false,
            supported_sources: sources,
        }
    }

    #[test_all]
    fn source_resolution_explicit_options_win() {
        let opts = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        };
        let route = route_with_sources(vec![SourceAsset::Bitcoin]);
        let result = decide_cross_chain_source(&route, None, Some(&opts), None, None);
        assert!(matches!(result, CrossChainSourceDecision::UseAsIs(Some(_))));
    }

    #[test_all]
    fn source_resolution_token_directly_supported() {
        let token = "USDB".to_string();
        let route = route_with_sources(vec![SourceAsset::Token {
            token_identifier: "USDB".to_string(),
        }]);
        let result = decide_cross_chain_source(&route, Some(&token), None, None, None);
        assert_eq!(result, CrossChainSourceDecision::UseAsIs(None));
    }

    #[test_all]
    fn source_resolution_auto_inject_to_bitcoin_for_active_stable_token() {
        let token = "USDB".to_string();
        let route = route_with_sources(vec![SourceAsset::Bitcoin]);
        let result = decide_cross_chain_source(&route, Some(&token), None, Some(&token), Some(150));
        match result {
            CrossChainSourceDecision::UseAsIs(Some(opts)) => {
                assert!(matches!(
                    opts.conversion_type,
                    ConversionType::ToBitcoin { ref from_token_identifier } if from_token_identifier == "USDB"
                ));
                assert_eq!(opts.max_slippage_bps, Some(150));
            }
            other => panic!("expected ToBitcoin auto-inject, got {other:?}"),
        }
    }

    #[test_all]
    fn source_resolution_token_not_supported_no_stable() {
        let token = "USDC".to_string();
        let route = route_with_sources(vec![SourceAsset::Bitcoin]);
        // Active stable is a different token → don't auto-inject.
        let other = "USDB".to_string();
        let result = decide_cross_chain_source(&route, Some(&token), None, Some(&other), None);
        assert_eq!(result, CrossChainSourceDecision::UseAsIs(None));
    }

    #[test_all]
    fn source_resolution_no_token_route_accepts_bitcoin_defers_to_stable() {
        let route = route_with_sources(vec![SourceAsset::Bitcoin]);
        let result = decide_cross_chain_source(&route, None, None, None, None);
        assert_eq!(result, CrossChainSourceDecision::DeferToStableBalance);
    }

    #[test_all]
    fn source_resolution_no_token_route_token_only() {
        let route = route_with_sources(vec![SourceAsset::Token {
            token_identifier: "USDB".to_string(),
        }]);
        let result = decide_cross_chain_source(&route, None, None, None, None);
        assert_eq!(result, CrossChainSourceDecision::UseAsIs(None));
    }

    #[test_all]
    fn source_resolution_explicit_options_override_token_and_stable() {
        // Even when token is supported AND it's the active stable token, an
        // explicit conversion_options from the caller wins.
        let token = "USDB".to_string();
        let explicit = ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "USDC".to_string(),
            },
            max_slippage_bps: Some(75),
            completion_timeout_secs: None,
        };
        let route = route_with_sources(vec![
            SourceAsset::Token {
                token_identifier: "USDB".to_string(),
            },
            SourceAsset::Bitcoin,
        ]);
        let result =
            decide_cross_chain_source(&route, Some(&token), Some(&explicit), Some(&token), None);
        match result {
            CrossChainSourceDecision::UseAsIs(Some(opts)) => {
                // Got the caller's options verbatim, not the auto-injected
                // ToBitcoin-from-USDB.
                assert!(matches!(
                    opts.conversion_type,
                    ConversionType::ToBitcoin { ref from_token_identifier } if from_token_identifier == "USDC"
                ));
                assert_eq!(opts.max_slippage_bps, Some(75));
            }
            other => panic!("explicit options should win; got {other:?}"),
        }
    }

    #[test_all]
    fn source_resolution_token_directly_supported_wins_over_stable_auto_inject() {
        // When the token is in the route's supported_sources AND it's the
        // active stable token, prefer the direct token path over auto-injecting
        // ToBitcoin.
        let token = "USDB".to_string();
        let route = route_with_sources(vec![
            SourceAsset::Token {
                token_identifier: "USDB".to_string(),
            },
            SourceAsset::Bitcoin,
        ]);
        let result = decide_cross_chain_source(&route, Some(&token), None, Some(&token), Some(150));
        assert_eq!(
            result,
            CrossChainSourceDecision::UseAsIs(None),
            "direct token send should be preferred when supported"
        );
    }

    #[test_all]
    fn source_resolution_stable_token_but_route_token_only() {
        // Token is the active stable, but route only accepts tokens (a
        // different one). Can't auto-inject ToBitcoin because the route
        // doesn't accept sats. Falls through to UseAsIs(None) and the
        // post-validation will surface the route-mismatch error.
        let token = "USDB".to_string();
        let route = route_with_sources(vec![SourceAsset::Token {
            token_identifier: "USDC".to_string(),
        }]);
        let result = decide_cross_chain_source(&route, Some(&token), None, Some(&token), Some(150));
        assert_eq!(result, CrossChainSourceDecision::UseAsIs(None));
    }

    // ---- validate_route_supports_effective_source ----

    #[test_all]
    fn validate_effective_source_no_token_no_conversion_route_accepts_bitcoin() {
        let route = route_with_sources(vec![SourceAsset::Bitcoin]);
        assert!(validate_route_supports_effective_source(&route, None, None).is_ok());
    }

    #[test_all]
    fn validate_effective_source_no_token_no_conversion_route_token_only_errors() {
        let route = route_with_sources(vec![SourceAsset::Token {
            token_identifier: "USDB".to_string(),
        }]);
        let err = validate_route_supports_effective_source(&route, None, None).unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(
            msg.contains("sats") && msg.contains("USDB"),
            "error should mention 'sats' and list supported token (got: {msg})"
        );
    }

    #[test_all]
    fn validate_effective_source_token_supported_directly() {
        let token = "USDB".to_string();
        let route = route_with_sources(vec![SourceAsset::Token {
            token_identifier: "USDB".to_string(),
        }]);
        assert!(validate_route_supports_effective_source(&route, Some(&token), None).is_ok());
    }

    #[test_all]
    fn validate_effective_source_token_unsupported_errors() {
        let token = "USDC".to_string();
        let route = route_with_sources(vec![
            SourceAsset::Token {
                token_identifier: "USDB".to_string(),
            },
            SourceAsset::Bitcoin,
        ]);
        let err = validate_route_supports_effective_source(&route, Some(&token), None).unwrap_err();
        let SdkError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(
            msg.contains("USDC") && msg.contains("USDB") && msg.contains("sats"),
            "error should mention unsupported token + supported list (got: {msg})"
        );
    }

    #[test_all]
    fn validate_effective_source_conversion_present_route_must_accept_sats() {
        // With conversion active, the effective source is sats regardless
        // of the user's token_identifier — so the route must accept sats.
        let token = "USDB".to_string();
        let opts = ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "USDB".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        };
        let route = route_with_sources(vec![SourceAsset::Bitcoin]);
        assert!(
            validate_route_supports_effective_source(&route, Some(&token), Some(&opts)).is_ok(),
            "conversion → sats should pass when route accepts sats"
        );
    }

    #[test_all]
    fn validate_effective_source_conversion_present_but_route_token_only_errors() {
        // Conversion makes effective source sats, but route only accepts tokens —
        // the user can't get there even with a conversion.
        let token = "USDB".to_string();
        let opts = ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "USDB".to_string(),
            },
            max_slippage_bps: None,
            completion_timeout_secs: None,
        };
        let route = route_with_sources(vec![SourceAsset::Token {
            token_identifier: "USDC".to_string(),
        }]);
        let err = validate_route_supports_effective_source(&route, Some(&token), Some(&opts))
            .unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }
}
