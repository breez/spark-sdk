use crate::{
    ConversionEstimate, ConversionOptions, ConversionPurpose, ConversionType, FeePolicy,
    SendPaymentMethod, WaitForPaymentIdentifier,
    error::SdkError,
    models::{ConversionStatus, SendPaymentRequest, SendPaymentResponse},
    persist::PaymentMetadata,
    sdk::BreezSdk,
    sdk::payments::send,
    token_conversion::{
        ConversionAmount, DEFAULT_CONVERSION_TIMEOUT_SECS, TokenConversionResponse,
    },
    utils::payments::get_payment_with_conversion_details,
};

/// Gets conversion options for a payment, auto-populating from stable balance config if needed.
///
/// Returns the provided options if set, or auto-populates from stable balance config
/// if configured and there's not enough sats balance to cover the payment.
async fn get_conversion_options_for_payment(
    sdk: &BreezSdk,
    options: Option<&ConversionOptions>,
    token_identifier: Option<&String>,
    payment_amount: u128,
) -> Result<Option<ConversionOptions>, SdkError> {
    if let Some(stable_balance) = &sdk.stable_balance {
        stable_balance
            .get_conversion_options(options, token_identifier, payment_amount)
            .await
            .map_err(Into::into)
    } else {
        Ok(options.cloned())
    }
}

/// Estimates a conversion for a payment, returning `None` when no conversion is needed.
///
/// For `AmountIn`: validates with the given options directly (caller knows what to convert).
/// For `MinAmountOut`: auto-populates conversion options from stable balance config when applicable.
pub(super) async fn estimate_conversion(
    sdk: &BreezSdk,
    request_options: Option<&ConversionOptions>,
    token_identifier: Option<&String>,
    conversion_amount: ConversionAmount,
) -> Result<Option<ConversionEstimate>, SdkError> {
    match conversion_amount {
        ConversionAmount::AmountIn(_) => sdk
            .token_converter
            .validate(request_options, token_identifier, conversion_amount)
            .await
            .map_err(Into::into),
        ConversionAmount::MinAmountOut(amount) => {
            let options =
                get_conversion_options_for_payment(sdk, request_options, token_identifier, amount)
                    .await?;
            sdk.token_converter
                .validate(options.as_ref(), token_identifier, conversion_amount)
                .await
                .map_err(Into::into)
        }
    }
}

/// Detects whether a token-conversion send should sweep the existing sat balance
/// alongside the converted output (the documented "send-all with stable balance"
/// behavior).
///
/// True iff: caller supplied a `token_identifier` matching the converter's
/// `from_token_identifier`, the requested `amount` equals the user's full token
/// balance, the fee policy is `FeesIncluded`, and stable balance is active for
/// the same token. A mismatch between the request `token_identifier` and the
/// conversion options' `from_token_identifier` returns an error.
async fn is_send_all(
    sdk: &BreezSdk,
    from_token_identifier: &str,
    token_identifier: Option<&String>,
    amount: u128,
    fee_policy: FeePolicy,
) -> Result<bool, SdkError> {
    let Some(token_id) = token_identifier else {
        return Ok(false);
    };
    if token_id != from_token_identifier {
        return Err(SdkError::Generic(
            "Request token identifier must match conversion options".to_string(),
        ));
    }
    let token_balances = sdk.spark_wallet.get_token_balances().await?;
    let token_balance = token_balances.get(token_id).map_or(0, |tb| tb.balance);
    let has_active_stable_token = match &sdk.stable_balance {
        Some(sb) => sb.get_active_token_identifier().await.as_ref() == Some(token_id),
        None => false,
    };
    Ok(amount == token_balance && fee_policy == FeePolicy::FeesIncluded && has_active_stable_token)
}

/// Estimates the sats available from a token→BTC conversion.
///
/// **Caller precondition**: only invoke when you're committed to a token
/// conversion (i.e. `conversion_options` is `ToBitcoin` and `amount > 0`).
/// Callers that may not be in conversion flow should branch upstream — see
/// [`lnurl::pay::prepare`] for an example.
///
/// Branches on `token_identifier`:
/// - **Set** → `amount` is in token base units; uses `AmountIn(amount)` (variable
///   sat output). For send-all, adds the existing sat balance to the conversion
///   output.
/// - **Not set** → `amount` is already in sats; uses `MinAmountOut(amount)` so
///   the converter is guaranteed to deliver at least `amount` sats or fail.
///   `estimated_sats == amount` in this case.
///
/// Returns `(estimated_sats, conversion_estimate)`. The returned `estimated_sats`
/// is the *raw* expected conversion output — callers that need a defensive lower
/// bound (e.g. LNURL invoice sizing on the `AmountIn` path) should apply their
/// own slippage buffer. `conversion_estimate` is `None` only when the converter
/// soft-refuses (callers must treat this as an error, see the token-denominated
/// prepare paths).
pub(in crate::sdk) async fn estimate_sats_from_token_conversion(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    token_identifier: Option<&String>,
    amount: u128,
    fee_policy: FeePolicy,
) -> Result<(u128, Option<ConversionEstimate>), SdkError> {
    let ConversionType::ToBitcoin {
        from_token_identifier,
    } = &conversion_options.conversion_type
    else {
        return Err(SdkError::Generic(
            "estimate_sats_from_token_conversion expects ToBitcoin conversion options".to_string(),
        ));
    };

    let is_send_all = is_send_all(
        sdk,
        from_token_identifier,
        token_identifier,
        amount,
        fee_policy,
    )
    .await?;

    // When token_identifier is provided, `amount` is in token units → AmountIn.
    // When it's omitted, `amount` is in sats → MinAmountOut (we want at least
    // that many sats out of the conversion).
    let (conversion_amount, estimated_sats_from_conversion) = if token_identifier.is_some() {
        let estimate = estimate_conversion(
            sdk,
            Some(conversion_options),
            token_identifier,
            ConversionAmount::AmountIn(amount),
        )
        .await?;
        let sats = estimate.as_ref().map_or(0, |e| e.amount_out);
        (estimate, sats)
    } else {
        let estimate = estimate_conversion(
            sdk,
            Some(conversion_options),
            token_identifier,
            ConversionAmount::MinAmountOut(amount),
        )
        .await?;
        // For MinAmountOut, the requested sats is the amount we asked for.
        (estimate, amount)
    };

    // For send-all, include existing sats balance — the actual send at execution
    // time will use the full post-conversion balance.
    let estimated_sats = if is_send_all {
        let sat_balance = u128::from(sdk.spark_wallet.get_balance().await?);
        estimated_sats_from_conversion.saturating_add(sat_balance)
    } else {
        estimated_sats_from_conversion
    };

    Ok((estimated_sats, conversion_amount))
}

/// Resolves the effective send amount and conversion estimate for a prepare flow
/// where the destination accepts sats directly (Spark address, Spark invoice).
///
/// - **Token conversion** (`token_identifier` set + `ToBitcoin` options): substitutes
///   `amount` with the post-conversion estimated sats, returns the `AmountIn` estimate.
/// - **Plain send with conversion options** (no `token_identifier`, sats `amount` +
///   options): keeps `amount` as-is, attaches a `MinAmountOut` estimate for display.
/// - **Plain send (no options)**: passes through unchanged with `None` estimate.
pub(super) async fn resolve_send_amount_with_conversion_estimate(
    sdk: &BreezSdk,
    conversion_options: Option<&ConversionOptions>,
    token_identifier: Option<&String>,
    amount: u128,
    fee_policy: FeePolicy,
) -> Result<(u128, Option<ConversionEstimate>), SdkError> {
    // Token-denominated: substitute `amount` with the post-conversion estimated sats.
    // Errors explicitly if the converter can't produce an estimate, since silently
    // falling through to the sats branch would reinterpret the user's token amount
    // as a sat amount.
    if let Some(opts) = conversion_options
        && is_token_denominated(Some(amount), Some(opts), token_identifier)
    {
        let (estimated_sats, conversion_estimate) =
            estimate_sats_from_token_conversion(sdk, opts, token_identifier, amount, fee_policy)
                .await?;
        if conversion_estimate.is_none() {
            return Err(SdkError::InvalidInput(
                "Token conversion is not available for the requested token and amount".to_string(),
            ));
        }
        return Ok((estimated_sats, conversion_estimate));
    }

    // Sats-denominated: keep `amount` as-is, attach a `MinAmountOut` estimate for display
    // (or `None` when no conversion options were set).
    let estimate = estimate_conversion(
        sdk,
        conversion_options,
        token_identifier,
        ConversionAmount::MinAmountOut(amount),
    )
    .await?;
    Ok((amount, estimate))
}

pub(super) async fn convert_token_send_payment_internal(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    request: &SendPaymentRequest,
    caller_amount_override: Option<u64>,
    suppress_payment_event: &mut bool,
) -> Result<SendPaymentResponse, SdkError> {
    // Suppress auto-convert while this send-with-conversion is in flight
    let _payment_guard = match &sdk.stable_balance {
        Some(sb) => Some(sb.acquire_payment_guard().await),
        None => None,
    };

    // Step 1: Execute the token conversion
    let (conversion_response, conversion_purpose, uses_amount_in) =
        execute_pre_send_conversion(sdk, conversion_options, request).await?;

    // Step 2: Early-link conversion children (self-transfer only)
    pre_link_conversion_children(sdk, &conversion_response, &conversion_purpose).await?;

    // Step 3: Trigger sync, wait for conversion, then send
    complete_conversion_and_send(
        sdk,
        conversion_options,
        &conversion_response,
        &conversion_purpose,
        request,
        uses_amount_in,
        caller_amount_override,
        suppress_payment_event,
    )
    .await
    // _payment_guard drops here, releasing the lock and waking the conversion worker
}

/// Executes the token conversion for the given payment method.
///
/// Returns the conversion response, purpose (self-transfer vs ongoing payment),
/// and whether the conversion used `AmountIn` (needed by `complete_conversion_and_send`
/// to compute the amount override).
///
/// Chooses the conversion direction based on whether the prepare used `AmountIn`
/// (user specified token amount, `amount == estimate.amount_out`) or `MinAmountOut`
/// (user specified sats). For `MinAmountOut`, the per-payment-method paths expand to
/// `MinAmountOut(amount + fees)` so the converter delivers enough to cover the send.
async fn execute_pre_send_conversion(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    request: &SendPaymentRequest,
) -> Result<(TokenConversionResponse, ConversionPurpose, bool), SdkError> {
    let amount = request.prepare_response.amount;

    // Extract the token identifier for the conversion.
    // For ToBitcoin, it's embedded in the conversion type.
    // For FromBitcoin, it comes from the prepare response's root token_identifier.
    let from_token_identifier = match &conversion_options.conversion_type {
        ConversionType::ToBitcoin {
            from_token_identifier,
        } => Some(from_token_identifier.clone()),
        ConversionType::FromBitcoin => request.prepare_response.token_identifier.clone(),
    };

    // AmountIn vs MinAmountOut at convert time (see uses_amount_in for the invariant).
    let uses_amount_in = uses_amount_in(
        amount,
        request.prepare_response.conversion_estimate.as_ref(),
    );
    let conversion_amount = if uses_amount_in {
        let token_amount = request
            .prepare_response
            .conversion_estimate
            .as_ref()
            .map(|e| e.amount_in)
            .ok_or(SdkError::InvalidInput(
                "Conversion estimate required for token conversion".to_string(),
            ))?;
        ConversionAmount::AmountIn(token_amount)
    } else {
        ConversionAmount::MinAmountOut(amount)
    };

    match &request.prepare_response.payment_method {
        SendPaymentMethod::SparkAddress { address, .. } => {
            let (response, purpose) = send::spark_address::convert_token(
                sdk,
                conversion_options,
                address,
                conversion_amount,
                from_token_identifier.as_ref(),
            )
            .await?;
            Ok((response, purpose, uses_amount_in))
        }
        SendPaymentMethod::SparkInvoice {
            spark_invoice_details,
            ..
        } => {
            let (response, purpose) = send::spark_invoice::convert_token(
                sdk,
                conversion_options,
                spark_invoice_details,
                conversion_amount,
                from_token_identifier.as_ref(),
            )
            .await?;
            Ok((response, purpose, uses_amount_in))
        }
        SendPaymentMethod::Bolt11Invoice {
            spark_transfer_fee_sats,
            lightning_fee_sats,
            invoice_details,
            ..
        } => {
            let (response, purpose) = send::bolt11::convert_token(
                sdk,
                conversion_options,
                invoice_details,
                *spark_transfer_fee_sats,
                *lightning_fee_sats,
                request,
                from_token_identifier.as_ref(),
                conversion_amount,
            )
            .await?;
            Ok((response, purpose, uses_amount_in))
        }
        SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
            let (response, purpose) = send::bitcoin_address::convert_token(
                sdk,
                conversion_options,
                address,
                fee_quote,
                request,
                from_token_identifier.as_ref(),
                conversion_amount,
            )
            .await?;
            Ok((response, purpose, uses_amount_in))
        }
        SendPaymentMethod::CrossChainAddress { .. } => {
            let (response, purpose) = send::cross_chain::convert_token(
                sdk,
                conversion_options,
                &request.prepare_response.payment_method,
                from_token_identifier.as_ref(),
                conversion_amount,
            )
            .await?;
            Ok((response, purpose, uses_amount_in))
        }
    }
}

/// Links conversion child payments to their parent to hide them from `list_payments`.
///
/// Only self-transfers are linked immediately (parent is the conversion receive, known upfront).
/// All other cases are deferred until after the actual send completes.
async fn pre_link_conversion_children(
    sdk: &BreezSdk,
    conversion_response: &TokenConversionResponse,
    conversion_purpose: &ConversionPurpose,
) -> Result<(), SdkError> {
    if *conversion_purpose == ConversionPurpose::SelfTransfer {
        sdk.storage
            .insert_payment_metadata(
                conversion_response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(conversion_response.received_payment_id.clone()),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}

/// Waits for conversion to complete, then sends the actual payment.
///
/// For self-transfers, returns immediately after conversion completes.
/// For ongoing payments, sends the actual payment and links any remaining children.
/// For `AmountIn` conversions, computes `amount_override = converted_sats + sats_change`
/// where `sats_change` is the difference between the prepare amount and the estimated
/// conversion output (representing any existing sat balance included at prepare time).
/// If `caller_amount_override` is provided (e.g. from the LNURL flow which handles
/// its own fee logic), it takes precedence over the computed override.
#[allow(clippy::too_many_arguments)]
async fn complete_conversion_and_send(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    conversion_response: &TokenConversionResponse,
    conversion_purpose: &ConversionPurpose,
    request: &SendPaymentRequest,
    uses_amount_in: bool,
    caller_amount_override: Option<u64>,
    suppress_payment_event: &mut bool,
) -> Result<SendPaymentResponse, SdkError> {
    // Wait for the received conversion payment to complete
    let payment = sdk
        .wait_for_incoming_payment(
            WaitForPaymentIdentifier::PaymentId(conversion_response.received_payment_id.clone()),
            conversion_options
                .completion_timeout_secs
                .unwrap_or(DEFAULT_CONVERSION_TIMEOUT_SECS),
        )
        .await
        .map_err(|e| {
            SdkError::Generic(format!("Timeout waiting for conversion to complete: {e}"))
        })?;

    // For self-transfers, suppress the event and return
    if *conversion_purpose == ConversionPurpose::SelfTransfer {
        *suppress_payment_event = true;
        return Ok(SendPaymentResponse { payment });
    }

    // Determine the amount to use for the actual send (see compute_amount_override
    // for the caller-override / AmountIn / MinAmountOut logic). The fallible
    // u128→u64 conversions are only needed (and only performed) on the AmountIn
    // path with no caller override, matching the original short-circuit.
    let (converted_sats, estimated_conversion_out) =
        if caller_amount_override.is_none() && uses_amount_in {
            let converted_sats: u64 = payment
                .amount
                .try_into()
                .map_err(|_| SdkError::Generic("Converted sats too large for u64".to_string()))?;
            let estimated_conversion_out: u64 = request
                .prepare_response
                .conversion_estimate
                .as_ref()
                .map_or(0, |e| e.amount_out)
                .try_into()
                .map_err(|_| SdkError::Generic("Estimated sats too large for u64".to_string()))?;
            (converted_sats, estimated_conversion_out)
        } else {
            (0, 0)
        };
    let amount_override = compute_amount_override(
        caller_amount_override,
        uses_amount_in,
        request.prepare_response.amount,
        estimated_conversion_out,
        converted_sats,
    );
    tracing::trace!(
        ?amount_override,
        uses_amount_in,
        converted_sats,
        estimated_conversion_out,
        prepared_amount = request.prepare_response.amount,
        fee_policy = ?request.prepare_response.fee_policy,
        "complete_conversion_and_send: computed amount_override"
    );

    // Now send the actual payment
    let response = Box::pin(send::send_internal(sdk, request, amount_override)).await?;

    // Link conversion children to the send payment (deferred linking)
    sdk.storage
        .insert_payment_metadata(
            conversion_response.sent_payment_id.clone(),
            PaymentMetadata {
                parent_payment_id: Some(response.payment.id.clone()),
                ..Default::default()
            },
        )
        .await?;
    sdk.storage
        .insert_payment_metadata(
            conversion_response.received_payment_id.clone(),
            PaymentMetadata {
                parent_payment_id: Some(response.payment.id.clone()),
                ..Default::default()
            },
        )
        .await?;

    // Persist Completed status on the actual send payment
    sdk.storage
        .insert_payment_metadata(
            response.payment.id.clone(),
            PaymentMetadata {
                conversion_status: Some(ConversionStatus::Completed),
                ..Default::default()
            },
        )
        .await?;

    // Fetch the updated payment with conversion details
    get_payment_with_conversion_details(response.payment.id, sdk.storage.clone())
        .await
        .map(|payment| SendPaymentResponse { payment })
}

/// Returns whether the conversion options request a token→sats conversion.
///
/// Used by [`is_token_denominated`] for routing decisions, and by per-type
/// prepare paths that need to know whether the destination sats will be
/// produced by a conversion (so leaf selection should be skipped at fee-quote
/// time — see `prepare/bitcoin_address.rs`).
pub(super) fn is_to_bitcoin(conversion_options: Option<&ConversionOptions>) -> bool {
    matches!(
        conversion_options,
        Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin { .. },
            ..
        })
    )
}

/// Returns the `ConversionPurpose` for a Spark-rail send: `SelfTransfer` when
/// the destination is our own identity (the conversion stays in-wallet),
/// otherwise an `OngoingPayment` toward the destination's payment request.
pub(super) fn conversion_purpose_for_identity(
    own_identity_pubkey: &str,
    target_identity_pubkey: &str,
    payment_request: String,
) -> ConversionPurpose {
    if target_identity_pubkey == own_identity_pubkey {
        ConversionPurpose::SelfTransfer
    } else {
        ConversionPurpose::OngoingPayment { payment_request }
    }
}

/// Returns the `token_identifier` to surface on a prepare response.
///
/// For `ToBitcoin` conversions the prepared output is sats, so the response
/// reflects the output denomination by clearing the (input) token identifier.
/// Otherwise the input token identifier is preserved.
pub(super) fn response_token_identifier(
    conversion_estimate: Option<&ConversionEstimate>,
    input_token_identifier: Option<String>,
) -> Option<String> {
    let is_to_bitcoin = matches!(
        conversion_estimate,
        Some(ConversionEstimate {
            options: ConversionOptions {
                conversion_type: ConversionType::ToBitcoin { .. },
                ..
            },
            ..
        })
    );
    if is_to_bitcoin {
        None
    } else {
        input_token_identifier
    }
}

/// Returns whether the request is token-denominated: the user supplied an
/// `amount` (in token base units), declared the source token via
/// `token_identifier`, and the conversion is `ToBitcoin`.
///
/// Used at the top of each per-type prepare to branch between the token-denominated
/// and sats-denominated paths explicitly, instead of probing further down. When
/// `token_identifier` is unset, the user's `amount` is in sats and the conversion
/// (if any) just sources the sats — that's the sats-denominated path.
pub(super) fn is_token_denominated(
    amount: Option<u128>,
    conversion_options: Option<&ConversionOptions>,
    token_identifier: Option<&String>,
) -> bool {
    amount.is_some() && token_identifier.is_some() && is_to_bitcoin(conversion_options)
}

/// Returns whether the prepare used `AmountIn` (user specified the token amount)
/// rather than `MinAmountOut` (user specified sats).
///
/// At prepare time the `AmountIn` path derives `amount` from `estimate.amount_out`
/// (plus any existing sat balance), so `amount >= amount_out`. The `MinAmountOut`
/// path guarantees `amount_out >= amount` (strictly greater when there's slack).
/// With no estimate the send isn't a conversion.
fn uses_amount_in(amount: u128, conversion_estimate: Option<&ConversionEstimate>) -> bool {
    conversion_estimate.is_some_and(|e| amount >= e.amount_out)
}

/// Computes the amount override for the actual send after a token conversion.
///
/// - A caller-provided override (e.g. the LNURL flow, which runs its own fee
///   logic) takes precedence.
/// - For `AmountIn` conversions the prepare amount bundled the estimated
///   conversion output plus any existing sat balance (`sats_change`); at send
///   time we use the *actual* converted sats plus that change, honoring the
///   prepare estimate while accounting for slippage. This unifies send-all
///   (`sats_change > 0`) and non-send-all (`sats_change == 0`).
/// - For `MinAmountOut` conversions the conversion guarantees ≥ the requested
///   sats, so no override is needed — send exactly the prepared amount.
fn compute_amount_override(
    caller_override: Option<u64>,
    uses_amount_in: bool,
    prepared_amount: u128,
    estimated_out: u64,
    converted_sats: u64,
) -> Option<u64> {
    if let Some(override_amount) = caller_override {
        return Some(override_amount);
    }
    if !uses_amount_in {
        return None;
    }
    let sats_change =
        u64::try_from(prepared_amount).map_or(0, |amount| amount.saturating_sub(estimated_out));
    Some(converted_sats.saturating_add(sats_change))
}

#[cfg(test)]
mod tests {
    use super::{
        compute_amount_override, conversion_purpose_for_identity, is_to_bitcoin,
        is_token_denominated, response_token_identifier, uses_amount_in,
    };
    use crate::{ConversionEstimate, ConversionOptions, ConversionPurpose, ConversionType};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn estimate_with_amount_out(amount_out: u128) -> ConversionEstimate {
        ConversionEstimate {
            options: ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: "token123".to_string(),
                },
                max_slippage_bps: None,
                completion_timeout_secs: None,
            },
            amount_in: 0,
            amount_out,
            fee: 0,
            amount_adjustment: None,
        }
    }

    fn from_bitcoin_estimate() -> ConversionEstimate {
        ConversionEstimate {
            options: ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
                completion_timeout_secs: None,
            },
            amount_in: 0,
            amount_out: 0,
            fee: 0,
            amount_adjustment: None,
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

    fn from_bitcoin_options() -> ConversionOptions {
        ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: None,
            completion_timeout_secs: None,
        }
    }

    // ============ is_to_bitcoin ============

    #[test_all]
    fn test_is_to_bitcoin_yes() {
        assert!(is_to_bitcoin(Some(&to_bitcoin_options())));
    }

    #[test_all]
    fn test_is_to_bitcoin_from_bitcoin() {
        assert!(!is_to_bitcoin(Some(&from_bitcoin_options())));
    }

    #[test_all]
    fn test_is_to_bitcoin_no_options() {
        assert!(!is_to_bitcoin(None));
    }

    // ============ is_token_denominated ============

    // ---- Happy path ----

    #[test_all]
    fn test_is_token_denominated_all_set() {
        // amount + token_id + ToBitcoin → token-denominated.
        let token_id = "token123".to_string();
        assert!(is_token_denominated(
            Some(1000),
            Some(&to_bitcoin_options()),
            Some(&token_id)
        ));
    }

    // ---- Missing inputs ----

    #[test_all]
    fn test_is_token_denominated_no_amount() {
        // No amount → never token-denominated (the user can't have given token units).
        let token_id = "token123".to_string();
        assert!(!is_token_denominated(
            None,
            Some(&to_bitcoin_options()),
            Some(&token_id)
        ));
    }

    #[test_all]
    fn test_is_token_denominated_no_token_identifier() {
        // Without `token_identifier` the user's `amount` is in sats, not tokens —
        // the conversion (if any) just sources the sats. That's the sats-denominated
        // path, even with `ToBitcoin` options set.
        assert!(!is_token_denominated(
            Some(1000),
            Some(&to_bitcoin_options()),
            None
        ));
    }

    #[test_all]
    fn test_is_token_denominated_no_conversion_options() {
        let token_id = "token123".to_string();
        assert!(!is_token_denominated(Some(1000), None, Some(&token_id)));
    }

    // ---- Wrong conversion direction ----

    #[test_all]
    fn test_is_token_denominated_from_bitcoin_is_sats_denominated() {
        // FromBitcoin (sats → tokens) is the sats-denominated path.
        let token_id = "token123".to_string();
        assert!(!is_token_denominated(
            Some(1000),
            Some(&from_bitcoin_options()),
            Some(&token_id)
        ));
    }

    // ============ uses_amount_in ============

    #[test_all]
    fn test_uses_amount_in_none_estimate() {
        assert!(!uses_amount_in(1000, None));
    }

    #[test_all]
    fn test_uses_amount_in_amount_ge_output() {
        // amount >= amount_out → AmountIn path.
        let estimate = estimate_with_amount_out(800);
        assert!(uses_amount_in(800, Some(&estimate)));
        assert!(uses_amount_in(1000, Some(&estimate)));
    }

    #[test_all]
    fn test_uses_amount_in_amount_lt_output() {
        // amount < amount_out → MinAmountOut path.
        let estimate = estimate_with_amount_out(800);
        assert!(!uses_amount_in(799, Some(&estimate)));
    }

    // ============ compute_amount_override ============

    // ---- Caller override takes precedence ----

    #[test_all]
    fn test_compute_amount_override_caller_override_wins() {
        // Caller override takes precedence regardless of the other inputs.
        assert_eq!(
            compute_amount_override(Some(1234), true, 5000, 4000, 4200),
            Some(1234)
        );
        assert_eq!(
            compute_amount_override(Some(1234), false, 0, 0, 0),
            Some(1234)
        );
    }

    // ---- MinAmountOut: no override needed ----

    #[test_all]
    fn test_compute_amount_override_min_amount_out_no_override() {
        // MinAmountOut conversion (uses_amount_in = false) → no override.
        assert_eq!(compute_amount_override(None, false, 1000, 800, 900), None);
    }

    // ---- AmountIn: actual sats + sats_change ----

    #[test_all]
    fn test_compute_amount_override_amount_in_no_change() {
        // AmountIn, non-send-all: prepared_amount == estimated_out → sats_change 0,
        // override is the actual converted sats.
        assert_eq!(
            compute_amount_override(None, true, 800, 800, 790),
            Some(790)
        );
    }

    #[test_all]
    fn test_compute_amount_override_amount_in_send_all() {
        // AmountIn, send-all: prepared_amount (1200) > estimated_out (800) →
        // sats_change 400 added to actual converted sats (810) → 1210.
        assert_eq!(
            compute_amount_override(None, true, 1200, 800, 810),
            Some(1210)
        );
    }

    // ---- Defensive: saturation/overflow paths ----

    #[test_all]
    fn test_compute_amount_override_estimated_out_exceeds_prepared() {
        // Defensive: if the estimate somehow exceeds the prepared amount,
        // sats_change saturates to 0 (no underflow) → override is the converted sats.
        assert_eq!(
            compute_amount_override(None, true, 700, 800, 790),
            Some(790)
        );
    }

    #[test_all]
    fn test_compute_amount_override_prepared_amount_exceeds_u64() {
        // prepared_amount > u64::MAX → the u64 conversion fails and sats_change
        // defaults to 0, so the override is just the converted sats.
        let prepared = u128::from(u64::MAX) + 1;
        assert_eq!(
            compute_amount_override(None, true, prepared, 0, 500),
            Some(500)
        );
    }

    // ============ response_token_identifier ============

    #[test_all]
    fn test_response_token_identifier_to_bitcoin_clears() {
        // ToBitcoin conversion outputs sats, so the response token_identifier
        // is cleared regardless of the input token identifier.
        let estimate = estimate_with_amount_out(1000);
        assert_eq!(
            response_token_identifier(Some(&estimate), Some("token123".to_string())),
            None
        );
    }

    #[test_all]
    fn test_response_token_identifier_from_bitcoin_preserves() {
        // FromBitcoin conversion outputs tokens — preserve the input token identifier.
        let estimate = from_bitcoin_estimate();
        assert_eq!(
            response_token_identifier(Some(&estimate), Some("token123".to_string())),
            Some("token123".to_string())
        );
    }

    #[test_all]
    fn test_response_token_identifier_no_estimate_preserves() {
        // No conversion — pass through the input token identifier.
        assert_eq!(
            response_token_identifier(None, Some("token123".to_string())),
            Some("token123".to_string())
        );
        assert_eq!(response_token_identifier(None, None), None);
    }

    // ============ conversion_purpose_for_identity ============

    #[test_all]
    fn test_conversion_purpose_self_transfer() {
        // Target == own → SelfTransfer.
        let purpose = conversion_purpose_for_identity("pubkey_a", "pubkey_a", "dest".to_string());
        assert!(matches!(purpose, ConversionPurpose::SelfTransfer));
    }

    #[test_all]
    fn test_conversion_purpose_ongoing_payment() {
        // Target != own → OngoingPayment with the destination payment_request.
        let purpose =
            conversion_purpose_for_identity("pubkey_a", "pubkey_b", "destination".to_string());
        match purpose {
            ConversionPurpose::OngoingPayment { payment_request } => {
                assert_eq!(payment_request, "destination");
            }
            _ => panic!("Expected OngoingPayment"),
        }
    }
}
