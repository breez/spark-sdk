use breez_sdk_common::input;

use crate::{
    ConversionEstimate, ConversionOptions, ConversionType, CrossChainFeeMode, CrossChainRoutePair,
    FeePolicy, SendPaymentMethod,
    cross_chain::{
        DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS, DEFAULT_TARGET_OVERPAY_BPS, MAX_CROSS_CHAIN_SLIPPAGE_BPS,
        MAX_TARGET_OVERPAY_BPS, MIN_CROSS_CHAIN_SLIPPAGE_BPS, MIN_TARGET_OVERPAY_BPS, SourceAsset,
        convert_destination_amount_to_sats, fetch_btc_usd_rate, rescale_decimals,
    },
    error::SdkError,
    models::PrepareSendPaymentResponse,
    sdk::BreezSdk,
    token_conversion::ConversionAmount,
};

use super::super::{conversion, validation};

/// Dispatcher-side overrides for response fields. `None` leaves the
/// provider's value through; `Some` replaces it with a user-facing figure
/// (e.g. AMM-derived token debit in place of provider-leg sats).
#[derive(Default)]
struct PrepareResponseOverrides {
    amount_in: Option<u128>,
    asset_amount_in: Option<u128>,
    fee_amount: Option<u128>,
}

/// Source-token ticker + decimals from the wallet's balances. Ticker drives
/// the USD-stable check; decimals drive the par-rescale.
struct SrcTokenMetadata {
    ticker: String,
    decimals: u32,
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
fn validate_address_family_against_route(
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

/// Resolves the target-overpay bps to apply on `FeesExcluded` conversion sends.
///
/// Same precedence as slippage: caller-supplied value (bounds-checked here),
/// then the config default, then the built-in default. Config defaults are
/// validated at SDK startup in `Config::validate`.
fn resolve_target_overpay_bps(
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
fn inflate_target_amount(amount: u128, overpay_bps: u32) -> u128 {
    if overpay_bps == 0 {
        return amount;
    }
    amount.saturating_add(amount.saturating_mul(u128::from(overpay_bps)) / 10_000)
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
    target_overpay_bps: Option<u32>,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    validate_request(amount, conversion_options.as_ref())?;

    let address_family = input::detect_address_family(address).ok_or_else(|| {
        SdkError::InvalidInput("Address is not a recognized cross-chain address".to_string())
    })?;
    validate_address_family_against_route(address_family, route)?;

    let provider_slippage_bps = resolve_slippage_bps(
        max_slippage_bps,
        sdk.config
            .cross_chain_config
            .as_ref()
            .and_then(|c| c.default_slippage_bps),
    )?;
    let overpay_bps = resolve_target_overpay_bps(
        target_overpay_bps,
        sdk.config
            .cross_chain_config
            .as_ref()
            .and_then(|c| c.default_target_overpay_bps),
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

    match (effective_conversion_options.as_ref(), fee_policy) {
        (None, _) => {
            prepare_sats_denominated(
                sdk,
                address,
                route,
                amount,
                token_identifier,
                provider_slippage_bps,
                overpay_bps,
                fee_policy,
            )
            .await
        }
        (Some(opts), FeePolicy::FeesIncluded) => {
            prepare_token_denominated_fees_included(
                sdk,
                address,
                route,
                amount,
                token_identifier.as_ref(),
                opts,
                provider_slippage_bps,
            )
            .await
        }
        (Some(opts), FeePolicy::FeesExcluded) => {
            prepare_token_denominated_fees_excluded(
                sdk,
                address,
                route,
                amount,
                token_identifier.as_ref(),
                opts,
                provider_slippage_bps,
                overpay_bps,
            )
            .await
        }
    }
}

/// No-conversion path: `amount` is in the route's source-asset units. The
/// provider's `prepared` flows straight to the response.
///
/// For `FeesExcluded` with a USD-stable token source bridging to a USD-stable
/// destination, inflates `amount` by `overpay_bps` so the provider sizes the
/// quote against a slightly higher target. `response.amount` keeps the
/// user's original input; `payment_method.amount_in` reflects the actual
/// debit after the provider's own fee scaling.
#[allow(clippy::too_many_arguments)]
async fn prepare_sats_denominated(
    sdk: &BreezSdk,
    address: &str,
    route: &CrossChainRoutePair,
    amount: u128,
    token_identifier: Option<String>,
    provider_slippage_bps: u32,
    overpay_bps: u32,
    fee_policy: FeePolicy,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    let provider_amount = resolve_direct_overpay_amount(amount, fee_policy, overpay_bps);
    tracing::debug!(
        provider = ?route.provider,
        chain = %route.chain,
        asset = %route.asset,
        amount,
        token_identifier = ?token_identifier,
        provider_slippage_bps,
        overpay_bps,
        provider_amount,
        "Cross-chain dispatcher: prepare_sats_denominated start"
    );

    let service = sdk.cross_chain_context.get(route.provider)?;
    let prepared = service
        .prepare(
            address,
            route,
            provider_amount,
            token_identifier.clone(),
            provider_slippage_bps,
            fee_policy.into(),
        )
        .await?;
    let response_token_identifier = conversion::response_token_identifier(None, token_identifier);
    Ok(build_response(
        prepared,
        amount,
        response_token_identifier,
        None,
        fee_policy,
        &PrepareResponseOverrides::default(),
    ))
}

/// Returns the amount to hand the provider on the no-conversion path.
/// Inflated when the policy is `FeesExcluded`: the caller asked for delivery
/// at-or-above target, so we pad the source to absorb provider slippage. The
/// route filter at the SDK aggregator ensures destinations are USD-pegged,
/// so the parity assumption holds for both BTC-source (sats fiat-inverted
/// inside the provider) and stable-token-source (parity rescale). `FeesIncluded`
/// ("send all") passes through unchanged.
fn resolve_direct_overpay_amount(amount: u128, fee_policy: FeePolicy, overpay_bps: u32) -> u128 {
    if overpay_bps == 0 || !matches!(fee_policy, FeePolicy::FeesExcluded) {
        return amount;
    }
    inflate_target_amount(amount, overpay_bps)
}

/// Conversion + `FeesIncluded` ("send all"): `amount` is the user's
/// source-token budget (USDB base units). The AMM `AmountIn` step converts
/// it to a sats budget that the provider treats as send-all; the recipient
/// gets the post-fee remainder.
async fn prepare_token_denominated_fees_included(
    sdk: &BreezSdk,
    address: &str,
    route: &CrossChainRoutePair,
    amount: u128,
    token_identifier: Option<&String>,
    conversion_options: &ConversionOptions,
    provider_slippage_bps: u32,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    // The conversion path always converts FROM a token; the authoritative id
    // lives on `conversion_options`. Upstream `validate_request` rejects the
    // `FromBitcoin` variant, so this match is structurally exhaustive.
    let ConversionType::ToBitcoin {
        from_token_identifier,
    } = &conversion_options.conversion_type
    else {
        return Err(SdkError::Generic(
            "Cross-chain conversion path requires ToBitcoin options".to_string(),
        ));
    };
    // Fetched up front: cheap wallet check before the provider quote.
    let src_meta = src_token_metadata(sdk, from_token_identifier).await?;

    let (sats, conversion_estimate) = estimate_and_validate_conversion(
        sdk,
        conversion_options,
        token_identifier,
        amount,
        FeePolicy::FeesIncluded,
    )
    .await?;

    let service = sdk.cross_chain_context.get(route.provider)?;
    let prepared = service
        .prepare(
            address,
            route,
            sats,
            None,
            provider_slippage_bps,
            CrossChainFeeMode::FeesIncluded,
        )
        .await?;

    // USD-stable pairs: surface the token-side debit. Non-stable: provider
    // values flow through.
    let overrides = compute_conversion_overrides(
        conversion_estimate.as_ref(),
        &src_meta,
        route.decimals.into(),
        prepared.estimated_out,
    )?;

    let response_token_identifier = conversion::response_token_identifier(
        conversion_estimate.as_ref(),
        token_identifier.cloned(),
    );
    Ok(build_response(
        prepared,
        sats,
        response_token_identifier,
        conversion_estimate,
        FeePolicy::FeesIncluded,
        &overrides,
    ))
}

/// Conversion + `FeesExcluded` ("fees on top"): `amount` is the user's
/// source-token budget (USDB base units). USD-stable parity is what lets
/// the same magnitude double as the destination delivery target — the
/// math below assumes that, and the non-USD gate enforces it.
///
/// Pipeline: fiat-invert the (parity-equivalent) destination target to a
/// sats target (no AMM spread), get the inflated invoice sats from the
/// provider, then reverse-estimate the AMM to find the wallet token debit.
/// Non-USD pairs downgrade to `FeesIncluded`.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn prepare_token_denominated_fees_excluded(
    sdk: &BreezSdk,
    address: &str,
    route: &CrossChainRoutePair,
    amount: u128,
    token_identifier: Option<&String>,
    conversion_options: &ConversionOptions,
    provider_slippage_bps: u32,
    overpay_bps: u32,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    tracing::debug!(
        provider = ?route.provider,
        chain = %route.chain,
        asset = %route.asset,
        amount,
        token_identifier = ?token_identifier,
        provider_slippage_bps,
        overpay_bps,
        "Cross-chain dispatcher: prepare_token_denominated_fees_excluded start"
    );

    // The conversion path always converts FROM a token; the authoritative id
    // lives on `conversion_options`. Upstream `validate_request` rejects the
    // `FromBitcoin` variant, so this match is structurally exhaustive.
    let ConversionType::ToBitcoin {
        from_token_identifier,
    } = &conversion_options.conversion_type
    else {
        return Err(SdkError::Generic(
            "Cross-chain conversion path requires ToBitcoin options".to_string(),
        ));
    };
    let src_meta = src_token_metadata(sdk, from_token_identifier).await?;

    // Pad the user's target upward so provider slippage doesn't land delivery
    // below the requested amount. Quoted against the inflated value; provider
    // returns the recipient as `prepared.estimated_out` (which therefore sits
    // slightly above the user's `amount`).
    let target_amount = inflate_target_amount(amount, overpay_bps);

    // Read through the cross-chain-scoped cache so this fetch shares the TTL
    // window with the providers' own rate lookups.
    let btc_usd = fetch_btc_usd_rate(sdk.cross_chain_context.fiat_service().as_ref()).await?;
    let sats_for_provider =
        convert_destination_amount_to_sats(target_amount, btc_usd, route.decimals.into())?;
    tracing::debug!(
        btc_usd,
        target_amount,
        sats_for_provider,
        "Cross-chain dispatcher: fiat-inverted to sats_for_provider"
    );

    let service = sdk.cross_chain_context.get(route.provider)?;
    let prepared = service
        .prepare(
            address,
            route,
            sats_for_provider,
            None,
            provider_slippage_bps,
            CrossChainFeeMode::FeesExcluded,
        )
        .await?;
    tracing::debug!(
        provider_amount_in = prepared.amount_in,
        provider_asset_amount_in = prepared.asset_amount_in,
        provider_estimated_out = prepared.estimated_out,
        provider_fee_amount = prepared.fee_amount,
        "Cross-chain dispatcher: provider returned"
    );

    // AMM must produce enough sats to cover the provider invoice AND the
    // source-leg transfer fee (Boltz LN routing; 0 for Orchestra).
    let amm_target_sats = prepared
        .amount_in
        .saturating_add(u128::from(prepared.source_transfer_fee_sats));
    let conversion_estimate = conversion::estimate_conversion(
        sdk,
        Some(conversion_options),
        token_identifier,
        ConversionAmount::MinAmountOut(amm_target_sats),
    )
    .await?;
    tracing::debug!(
        amm_target_sats,
        source_transfer_fee_sats = prepared.source_transfer_fee_sats,
        amm_amount_in = ?conversion_estimate.as_ref().map(|e| e.amount_in),
        amm_amount_out = ?conversion_estimate.as_ref().map(|e| e.amount_out),
        amm_fee = ?conversion_estimate.as_ref().map(|e| e.fee),
        "Cross-chain dispatcher: AMM MinAmountOut reverse-estimate"
    );

    // Fail fast on insufficient balance: the AMM doesn't gate on user funds.
    if let Some(estimate) = conversion_estimate.as_ref()
        && let ConversionType::ToBitcoin {
            from_token_identifier,
        } = &estimate.options.conversion_type
    {
        let balances = sdk.spark_wallet.get_token_balances().await?;
        let have = balances
            .get(from_token_identifier)
            .map_or(0u128, |b| b.balance);
        if have < estimate.amount_in {
            tracing::warn!(
                token = %from_token_identifier,
                have,
                need = estimate.amount_in,
                "Cross-chain dispatcher: insufficient token balance"
            );
            return Err(SdkError::InvalidInput(format!(
                "Insufficient {from_token_identifier} balance for cross-chain conversion: \
                 have {have}, need {} (= {amount} target + AMM/provider spread + bridge fees).",
                estimate.amount_in
            )));
        }
    }

    // Reuse `src_meta` from the stable-pair gate above (avoids a second wallet hit).
    let overrides = compute_conversion_overrides(
        conversion_estimate.as_ref(),
        &src_meta,
        route.decimals.into(),
        prepared.estimated_out,
    )?;

    // response.amount = provider invoice. LN routing lives on
    // source_transfer_fee_sats; send-side convert_token folds it in.
    let response_token_identifier = conversion::response_token_identifier(
        conversion_estimate.as_ref(),
        token_identifier.cloned(),
    );
    let provider_amount_in = prepared.amount_in;
    Ok(build_response(
        prepared,
        provider_amount_in,
        response_token_identifier,
        conversion_estimate,
        FeePolicy::FeesExcluded,
        &overrides,
    ))
}

/// Errors if `token_id` isn't in the wallet's balances — the caller is
/// proceeding under the assumption that the source token exists.
async fn src_token_metadata(sdk: &BreezSdk, token_id: &str) -> Result<SrcTokenMetadata, SdkError> {
    let balances = sdk.spark_wallet.get_token_balances().await?;
    let tb = balances.get(token_id).ok_or_else(|| {
        SdkError::InvalidInput(format!(
            "Source token {token_id} not found in wallet balances"
        ))
    })?;
    Ok(SrcTokenMetadata {
        ticker: tb.token_metadata.ticker.clone(),
        decimals: tb.token_metadata.decimals,
    })
}

/// Surfaces the token-side debit on the response. Routes are filtered to
/// USD-pegged destinations at `get_cross_chain_routes`, so the par-rescale
/// always holds. Missing estimate (no AMM step) defers to provider values.
/// `fee_amount = asset_amount_in - provider_estimated_out`.
fn compute_conversion_overrides(
    estimate: Option<&ConversionEstimate>,
    src_meta: &SrcTokenMetadata,
    dest_decimals: u32,
    provider_estimated_out: u128,
) -> Result<PrepareResponseOverrides, SdkError> {
    let Some(estimate) = estimate else {
        tracing::debug!("Cross-chain dispatcher: no AMM estimate; deferring to provider amounts");
        return Ok(PrepareResponseOverrides::default());
    };
    let asset_amount_in = rescale_decimals(estimate.amount_in, src_meta.decimals, dest_decimals)?;
    let fee_amount = asset_amount_in.saturating_sub(provider_estimated_out);
    tracing::debug!(
        src_ticker = %src_meta.ticker,
        src_decimals = src_meta.decimals,
        user_token_amount_in = estimate.amount_in,
        asset_amount_in,
        response_fee_amount = fee_amount,
        "Cross-chain dispatcher: overrides computed"
    );
    Ok(PrepareResponseOverrides {
        amount_in: Some(estimate.amount_in),
        asset_amount_in: Some(asset_amount_in),
        fee_amount: Some(fee_amount),
    })
}

fn build_response(
    prepared: crate::cross_chain::CrossChainPrepared,
    response_amount: u128,
    response_token_identifier: Option<String>,
    conversion_estimate: Option<ConversionEstimate>,
    fee_policy: FeePolicy,
    overrides: &PrepareResponseOverrides,
) -> PrepareSendPaymentResponse {
    let amount_in = overrides.amount_in.unwrap_or(prepared.amount_in);
    let asset_amount_in = overrides
        .asset_amount_in
        .unwrap_or(prepared.asset_amount_in);
    let fee_amount = overrides.fee_amount.unwrap_or(prepared.fee_amount);
    PrepareSendPaymentResponse {
        payment_method: SendPaymentMethod::CrossChainAddress {
            route: prepared.pair,
            recipient_address: prepared.recipient_address,
            amount_in,
            asset_amount_in,
            estimated_out: prepared.estimated_out,
            fee_amount,
            service_fee_amount: prepared.service_fee_amount,
            service_fee_asset: prepared.service_fee_asset,
            source_transfer_fee_sats: prepared.source_transfer_fee_sats,
            fee_mode: prepared.fee_mode,
            expires_at: prepared.expires_at,
            provider_context: prepared.provider_context,
        },
        amount: response_amount,
        token_identifier: response_token_identifier,
        conversion_estimate,
        fee_policy,
    }
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
        .and_then(|sb| sb.core.config.max_slippage_bps);

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
        // 25 bps on 1_000_000 → 1_000_000 + 2_500 = 1_002_500.
        assert_eq!(inflate_target_amount(1_000_000, 25), 1_002_500);
    }

    #[test_all]
    fn inflate_target_amount_truncates_sub_unit_pad() {
        // 25 bps on 100 → 100 + (100 * 25 / 10_000) = 100 + 0 (integer floor).
        // Acceptable: pad is sub-unit for tiny amounts.
        assert_eq!(inflate_target_amount(100, 25), 100);
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

    // ---- compute_conversion_overrides ----

    fn estimate_with_amount_in(amount_in: u128, amount_out: u128) -> ConversionEstimate {
        ConversionEstimate {
            options: ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: "tok".to_string(),
                },
                max_slippage_bps: None,
                completion_timeout_secs: None,
            },
            amount_in,
            amount_out,
            fee: 0,
            amount_adjustment: None,
        }
    }

    #[test_all]
    fn compute_overrides_returns_default_when_estimate_is_none() {
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 6,
        };
        let overrides =
            compute_conversion_overrides(None, &src_meta, 6, 1_000_000).expect("pure helper");
        assert!(overrides.amount_in.is_none());
        assert!(overrides.asset_amount_in.is_none());
        assert!(overrides.fee_amount.is_none());
    }

    #[test_all]
    fn compute_overrides_stable_pair_populates_all_three_fields() {
        let est = estimate_with_amount_in(1_020_434, 1_002_502);
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 6,
        };
        let overrides = compute_conversion_overrides(Some(&est), &src_meta, 6, 1_000_000).unwrap();
        assert_eq!(overrides.amount_in, Some(1_020_434));
        // src/dest both 6 decimals → rescale is identity.
        assert_eq!(overrides.asset_amount_in, Some(1_020_434));
        // fee = asset_amount_in - provider_estimated_out
        assert_eq!(overrides.fee_amount, Some(20_434));
    }

    #[test_all]
    fn compute_overrides_rescales_decimals_when_source_dest_differ() {
        // Hypothetical USDB at 8 decimals → USDC at 6 decimals.
        let est = estimate_with_amount_in(102_043_400, 0);
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 8,
        };
        let overrides = compute_conversion_overrides(Some(&est), &src_meta, 6, 1_000_000).unwrap();
        // 102_043_400 / 10^2 = 1_020_434
        assert_eq!(overrides.asset_amount_in, Some(1_020_434));
        assert_eq!(overrides.fee_amount, Some(20_434));
    }

    #[test_all]
    fn compute_overrides_saturating_sub_clamps_when_delivery_over_input() {
        // Edge: AMM happens to over-deliver vs provider's estimated_out.
        let est = estimate_with_amount_in(1_000_000, 0);
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 6,
        };
        let overrides = compute_conversion_overrides(Some(&est), &src_meta, 6, 2_000_000).unwrap();
        assert_eq!(
            overrides.fee_amount,
            Some(0),
            "saturating_sub must clamp at 0 instead of underflowing"
        );
    }

    // ---- build_response: ToBitcoin convention ----

    fn mk_prepared(
        amount_in: u128,
        source_transfer_fee_sats: u64,
    ) -> crate::cross_chain::CrossChainPrepared {
        crate::cross_chain::CrossChainPrepared {
            amount_in,
            asset_amount_in: amount_in,
            estimated_out: amount_in.saturating_sub(30),
            fee_amount: 30,
            service_fee_amount: 20,
            service_fee_asset: None,
            source_transfer_fee_sats,
            fee_mode: CrossChainFeeMode::FeesExcluded,
            expires_at: "2099-01-01T00:00:00Z".to_string(),
            pair: CrossChainRoutePair {
                provider: crate::cross_chain::CrossChainProvider::Orchestra,
                chain: "base".to_string(),
                chain_id: Some("8453".to_string()),
                asset: "USDC".to_string(),
                contract_address: None,
                decimals: 6,
                exact_out_eligible: false,
                supported_sources: vec![],
            },
            recipient_address: "0xabc".to_string(),
            token_identifier: None,
            provider_context: crate::cross_chain::CrossChainProviderContext::Orchestra {
                quote_id: "q1".to_string(),
                deposit_address: "sp1...".to_string(),
                deposit_amount: amount_in,
            },
        }
    }

    #[test_all]
    fn response_token_identifier_clears_on_to_bitcoin_for_cross_chain() {
        // ToBitcoin: cleared (outbound leg is sats). No conversion: passthrough.
        let est = estimate_with_amount_in(1_015_000, 1_050);
        let cleared = conversion::response_token_identifier(Some(&est), Some("USDB".to_string()));
        assert!(cleared.is_none());

        let preserved = conversion::response_token_identifier(None, Some("USDB".to_string()));
        assert_eq!(preserved, Some("USDB".to_string()));
    }

    /// Helper for `build_response` assertions: extracts the relevant fields
    /// off the `CrossChainAddress` variant. Panics if the variant doesn't
    /// match.
    fn extract_cross_chain(method: &SendPaymentMethod) -> (u128, u128, u128, u64) {
        let SendPaymentMethod::CrossChainAddress {
            amount_in,
            asset_amount_in,
            fee_amount,
            source_transfer_fee_sats,
            ..
        } = method
        else {
            panic!("expected CrossChainAddress");
        };
        (
            *amount_in,
            *asset_amount_in,
            *fee_amount,
            *source_transfer_fee_sats,
        )
    }

    #[test_all]
    fn build_response_feesexcluded_orchestra_shape_applies_token_debit_override() {
        // Orchestra shape: source_transfer_fee_sats = 0.
        let prepared = mk_prepared(1_050, 0);
        let est = estimate_with_amount_in(1_020_000, 1_050);
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 6,
        };
        let overrides =
            compute_conversion_overrides(Some(&est), &src_meta, 6, prepared.estimated_out)
                .expect("pure helper");
        let response_token_identifier =
            conversion::response_token_identifier(Some(&est), Some("USDB".to_string()));
        let resp = build_response(
            prepared,
            1_050,
            response_token_identifier,
            Some(est),
            FeePolicy::FeesExcluded,
            &overrides,
        );
        assert_eq!(resp.amount, 1_050, "response.amount = provider invoice");
        assert!(resp.token_identifier.is_none(), "ToBitcoin clears it");
        let (amount_in, asset_amount_in, fee_amount, sttf) =
            extract_cross_chain(&resp.payment_method);
        assert_eq!(amount_in, 1_020_000, "amount_in = USDB debit");
        assert_eq!(
            asset_amount_in, 1_020_000,
            "USDB in USDC base units (parity)"
        );
        // fee = USDB_in_USDC_units - provider_estimated_out = 1_020_000 - (1_050 - 30)
        assert_eq!(fee_amount, 1_020_000 - 1_020);
        assert_eq!(sttf, 0);
    }

    #[test_all]
    fn build_response_feesexcluded_boltz_shape_applies_token_debit_override_and_carries_ln_fee() {
        // Boltz shape: non-zero source_transfer_fee_sats for LN routing.
        let prepared = mk_prepared(1_050, 25);
        let est = estimate_with_amount_in(1_020_000, 1_075);
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 6,
        };
        let overrides =
            compute_conversion_overrides(Some(&est), &src_meta, 6, prepared.estimated_out)
                .expect("pure helper");
        let response_token_identifier =
            conversion::response_token_identifier(Some(&est), Some("USDB".to_string()));
        let resp = build_response(
            prepared,
            1_050,
            response_token_identifier,
            Some(est),
            FeePolicy::FeesExcluded,
            &overrides,
        );
        assert_eq!(resp.amount, 1_050, "response.amount = invoice only");
        assert!(resp.token_identifier.is_none());
        let (amount_in, asset_amount_in, fee_amount, sttf) =
            extract_cross_chain(&resp.payment_method);
        assert_eq!(amount_in, 1_020_000, "amount_in = USDB debit (override)");
        assert_eq!(asset_amount_in, 1_020_000);
        assert_eq!(fee_amount, 1_020_000 - 1_020);
        assert_eq!(
            sttf, 25,
            "LN routing fee carried separately for send-side MinAmountOut expansion"
        );
    }

    #[test_all]
    fn build_response_feesincluded_conversion_applies_token_debit_override_symmetrically() {
        // FeesIncluded conversion: response.amount is the user's gross sats
        // budget; payment_method still carries the USDB debit override.
        let prepared = mk_prepared(1_050, 25);
        let est = estimate_with_amount_in(1_020_000, 1_075);
        let src_meta = SrcTokenMetadata {
            ticker: "USDB".to_string(),
            decimals: 6,
        };
        let overrides =
            compute_conversion_overrides(Some(&est), &src_meta, 6, prepared.estimated_out)
                .expect("pure helper");
        let response_token_identifier =
            conversion::response_token_identifier(Some(&est), Some("USDB".to_string()));
        let resp = build_response(
            prepared,
            1_075,
            response_token_identifier,
            Some(est),
            FeePolicy::FeesIncluded,
            &overrides,
        );
        assert_eq!(
            resp.amount, 1_075,
            "FeesIncluded response.amount = user's gross sats budget"
        );
        assert!(resp.token_identifier.is_none());
        let (amount_in, asset_amount_in, fee_amount, sttf) =
            extract_cross_chain(&resp.payment_method);
        assert_eq!(amount_in, 1_020_000, "amount_in = USDB debit (override)");
        assert_eq!(asset_amount_in, 1_020_000);
        assert_eq!(fee_amount, 1_020_000 - 1_020);
        assert_eq!(sttf, 25);
    }
}
