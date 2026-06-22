use std::str::FromStr;

use platform_utils::time::Duration;
use platform_utils::tokio;
use spark_wallet::{LightningSendContext, TransferId};
use tokio::sync::oneshot;
use tracing::{Instrument, error, info};

use crate::{
    Bolt11InvoiceDetails, ConversionOptions, ConversionPurpose, FeePolicy, PaymentStatus,
    SendPaymentOptions,
    error::SdkError,
    models::{Payment, PaymentDetails, SendPaymentRequest, SendPaymentResponse, TransferContext},
    sdk::BreezSdk,
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::payments::record_payment_update,
};

/// Reconstructs the spark-wallet send context from the public [`TransferContext`]
/// so `complete_lightning_send` can resume the prepared (Lightning) send.
fn lightning_send_context_from(tc: &TransferContext) -> Result<LightningSendContext, SdkError> {
    let transfer_id: TransferId = tc
        .transfer_id
        .parse()
        .map_err(|e| SdkError::Generic(format!("invalid transfer id in transfer context: {e}")))?;
    let leaf_ids = tc
        .leaf_ids
        .iter()
        .map(|id| id.parse::<spark_wallet::TreeNodeId>())
        .collect::<Result<Vec<_>, String>>()
        .map_err(|e| SdkError::Generic(format!("invalid leaf id in transfer context: {e}")))?;
    Ok(LightningSendContext::Lightning {
        invoice: tc.invoice.clone(),
        amount_to_send: tc.amount_to_send_sats,
        total_amount_sat: tc.total_amount_sats,
        transfer_id,
        leaf_ids: Some(leaf_ids),
    })
}

#[allow(clippy::too_many_lines)]
pub(super) async fn send(
    sdk: &BreezSdk,
    invoice_details: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    lightning_fee_sats: u64,
    request: &SendPaymentRequest,
    amount_override: Option<u64>,
    amount: u128,
) -> Result<SendPaymentResponse, SdkError> {
    // Determine routing preference and actual fee before calculating the send amount,
    // so FeesIncluded deducts the correct fee (Spark=0 vs Lightning).
    let (prefer_spark, completion_timeout_secs) = match request.options {
        Some(SendPaymentOptions::Bolt11Invoice {
            prefer_spark,
            completion_timeout_secs,
        }) => (prefer_spark, completion_timeout_secs),
        _ => (sdk.config.prefer_spark_over_lightning, None),
    };
    let is_spark_route = prefer_spark && spark_transfer_fee_sats.is_some();
    let fee_sats = if is_spark_route {
        spark_transfer_fee_sats.unwrap_or(0)
    } else {
        lightning_fee_sats
    };

    // Handle FeesIncluded: deduct fees from the total balance.
    // Applies to both amountless invoices and fixed-amount invoices with amount_override
    // (send-all-with-conversion via LNURL — overpays the invoice to drain the wallet).
    let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;
    let amount_to_send = if is_fees_included
        && (invoice_details.amount_msat.is_none() || amount_override.is_some())
    {
        let total_sats: u64 = match amount_override {
            Some(sat_balance) => sat_balance,
            None => amount.try_into()?,
        };
        // Spark route: deduct known fee directly (often 0).
        // Lightning route: re-estimate fees via calculate_fees_included_amount
        // which handles fee changes between prepare and send.
        let amt = if is_spark_route {
            total_sats.saturating_sub(fee_sats)
        } else {
            calculate_fees_included_amount(
                sdk,
                &invoice_details.invoice.bolt11,
                total_sats,
                fee_sats,
            )
            .await?
        };
        Some(u128::from(amt))
    } else {
        match amount_override {
            Some(amt) => Some(amt.into()),
            None => match invoice_details.amount_msat {
                Some(_) => None,
                None => Some(amount),
            },
        }
    };
    let transfer_id = request
        .idempotency_key
        .as_ref()
        .map(|idempotency_key| TransferId::from_str(idempotency_key))
        .transpose()?;

    let payment_response = if let Some(transfer_context) = &request.transfer_context {
        // Resume a gated send: complete from the pinned context (re-reserving its
        // exact leaves) rather than preparing afresh.
        Box::pin(
            sdk.spark_wallet
                .complete_lightning_send(lightning_send_context_from(transfer_context)?),
        )
        .await?
    } else {
        Box::pin(
            sdk.spark_wallet.pay_lightning_invoice(
                &invoice_details.invoice.bolt11,
                amount_to_send
                    .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                    .transpose()?,
                Some(fee_sats),
                prefer_spark,
                transfer_id,
            ),
        )
        .await?
    };
    let completion_timeout_secs = completion_timeout_secs.unwrap_or(0);
    let payment = match payment_response.lightning_payment {
        Some(lightning_payment) => {
            let ssp_id = lightning_payment.id.clone();
            let htlc_details = payment_response
                .transfer
                .htlc_preimage_request
                .ok_or_else(|| {
                    SdkError::Generic("Missing HTLC details for Lightning send payment".to_string())
                })?
                .try_into()?;
            let payment = Payment::from_lightning(
                lightning_payment,
                amount,
                payment_response.transfer.id.to_string(),
                htlc_details,
            )?;
            let completion_rx = poll_lightning_send_payment(sdk, &payment, ssp_id);
            if completion_timeout_secs == 0 {
                payment
            } else {
                // Wait up to the caller's timeout for the background
                // poll to signal completion. The poll keeps running in
                // either branch — it still emits `PaymentSucceeded`
                // when terminal — so dropping the receiver on timeout
                // is harmless. We fall back to the pre-confirmation
                // payment if the wait times out or the channel closes
                // (e.g. missing HTLC details).
                tokio::time::timeout(
                    Duration::from_secs(completion_timeout_secs.into()),
                    completion_rx,
                )
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(payment)
            }
        }
        // Spark-routed Lightning sends complete synchronously inside
        // `pay_lightning_invoice` — there is no SSP-side state to poll,
        // so `completion_timeout_secs` is ignored for this branch and
        // the payment is returned with whatever status the transfer
        // already has.
        None => payment_response.transfer.try_into()?,
    };

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(SendPaymentResponse { payment })
}

/// For `FeesIncluded` + amountless Bolt11: calculates the amount to send
/// (`receiver_amount` + any overpayment from fee decrease).
async fn calculate_fees_included_amount(
    sdk: &BreezSdk,
    invoice: &str,
    user_amount: u64,
    stored_fee: u64,
) -> Result<u64, SdkError> {
    let receiver_amount = user_amount.saturating_sub(stored_fee);
    if receiver_amount == 0 {
        return Err(SdkError::InvalidInput(
            "Amount too small to cover fees".to_string(),
        ));
    }

    // Re-estimate current fee for receiver amount
    let current_fee = sdk
        .spark_wallet
        .fetch_lightning_send_fee_estimate(invoice, Some(receiver_amount))
        .await?;

    let overpayment = fee_overpayment(stored_fee, current_fee)?;
    if overpayment > 0 {
        info!(
            overpayment_sats = overpayment,
            stored_fee_sats = stored_fee,
            current_fee_sats = current_fee,
            "FeesIncluded fee overpayment applied for Bolt11"
        );
    }

    Ok(receiver_amount.saturating_add(overpayment))
}

/// Pure kernel for the `FeesIncluded` fee-reconciliation shared by the Bolt11
/// and LNURL-pay send paths.
///
/// Given the fee stored at prepare time and the fee re-estimated at send time,
/// returns the allowed overpayment (`stored - current`). Fails if the fee
/// increased since prepare, or if the overpayment exceeds the cap of
/// `current_fee.max(1)` (allow up to 100% of the actual fee, minimum 1 sat).
pub(in crate::sdk) fn fee_overpayment(stored_fee: u64, current_fee: u64) -> Result<u64, SdkError> {
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

/// Spawns the background poll that watches an outgoing Lightning send to
/// completion. Returns a receiver that resolves to the terminal `Payment`
/// when the SSP reports a non-`Pending` status, so callers can `await`
/// completion synchronously with their own timeout.
fn poll_lightning_send_payment(
    sdk: &BreezSdk,
    payment: &Payment,
    ssp_id: String,
) -> oneshot::Receiver<Payment> {
    const MAX_POLL_ATTEMPTS: u32 = 20;
    let payment_id = payment.id.clone();
    let (tx, rx) = oneshot::channel();
    info!("Polling lightning send payment {}", payment_id);

    let Some(htlc_details) = payment.details.as_ref().and_then(|d| match d {
        PaymentDetails::Lightning { htlc_details, .. } => Some(htlc_details.clone()),
        _ => None,
    }) else {
        error!("Missing HTLC details for lightning send payment {payment_id}, skipping polling");
        return rx;
    };
    let spark_wallet = sdk.spark_wallet.clone();
    let storage = sdk.storage.clone();
    let event_emitter = sdk.event_emitter.clone();
    let payment = payment.clone();
    let payment_id = payment_id.clone();
    let mut shutdown = sdk.shutdown_sender.subscribe();
    let span = tracing::Span::current();

    tokio::spawn(async move {
        // Drive the poll loop until we either reach a terminal status,
        // hit the attempt cap, or get a shutdown signal.
        let terminal_payment: Option<Payment> = 'poll: {
            for i in 0..MAX_POLL_ATTEMPTS {
                info!(
                    "Polling lightning send payment {} attempt {}",
                    payment_id, i
                );
                tokio::select! {
                    _ = shutdown.changed() => {
                        info!("Shutdown signal received");
                        break 'poll None;
                    },
                    p = spark_wallet.fetch_lightning_send_payment(&ssp_id) => {
                        if let Ok(Some(p)) = p && let Ok(payment) = Payment::from_lightning(p.clone(), payment.amount, payment.id.clone(), htlc_details.clone()) {
                            info!("Polling payment status = {} {:?}", payment.status, p.status);
                            if payment.status != PaymentStatus::Pending {
                                info!("Polling payment completed status = {}", payment.status);
                                break 'poll Some(payment);
                            }
                        }

                        let sleep_time = if i < 5 {
                            Duration::from_secs(1)
                        } else {
                            Duration::from_secs(i.into())
                        };
                        tokio::time::sleep(sleep_time).await;
                    }
                }
            }
            None
        };

        let Some(payment) = terminal_payment else {
            return;
        };

        let _ = tx.send(payment.clone());
        record_payment_update(&storage, &event_emitter, payment, true).await;
    }.instrument(span));

    rx
}

/// Runs the token conversion for a Bolt11 send, returning the conversion response
/// and its `OngoingPayment` purpose. `AmountIn` passes through; `MinAmountOut` is
/// expanded to cover the routing fee (Spark transfer when preferred and available,
/// otherwise Lightning) so the converter delivers enough to complete the send.
#[expect(clippy::too_many_arguments)]
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    invoice_details: &Bolt11InvoiceDetails,
    spark_transfer_fee_sats: Option<u64>,
    lightning_fee_sats: u64,
    request: &SendPaymentRequest,
    token_identifier: Option<&String>,
    conversion_amount: ConversionAmount,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let purpose = ConversionPurpose::OngoingPayment {
        payment_request: invoice_details.invoice.bolt11.clone(),
    };

    let conversion_amount = match conversion_amount {
        ConversionAmount::AmountIn(_) => conversion_amount,
        ConversionAmount::MinAmountOut(amount) => {
            // Determine the fee to be used based on preference
            let fee_sats = match request.options {
                Some(SendPaymentOptions::Bolt11Invoice { prefer_spark, .. }) => {
                    match (prefer_spark, spark_transfer_fee_sats) {
                        (true, Some(fee)) => fee,
                        _ => lightning_fee_sats,
                    }
                }
                _ => lightning_fee_sats,
            };
            // The absolute minimum amount out is the lightning invoice amount plus fee
            ConversionAmount::MinAmountOut(amount.saturating_add(u128::from(fee_sats)))
        }
    };

    let response = sdk
        .token_converter
        .convert(
            sdk.event_emitter.clone(),
            conversion_options,
            &purpose,
            token_identifier,
            conversion_amount,
            None,
        )
        .await?;
    Ok((response, purpose))
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
