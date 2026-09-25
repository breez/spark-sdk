//! Conversion logic for stable balance.
//!
//! Contains the actual conversion methods called by the unified worker:
//! - `per_receive_convert`: converts individual received payments
//! - `auto_convert`: batch converts accumulated BTC above threshold
//! - `deactivation_convert`: converts all tokens back to BTC on deactivation

use std::sync::atomic::Ordering;

use tracing::{debug, info, warn};

use crate::events::EventEmitter;
use crate::models::{ConversionStatus, Payment, PaymentDetails};
use crate::persist::PaymentMetadata;
use crate::token_conversion::{
    ConversionAmount, ConversionError, ConversionOptions, ConversionPurpose, ConversionType,
    FetchConversionLimitsRequest, TokenConversionResponse,
};
use crate::utils::conversions::extract_conversion_info;
use crate::utils::payments::insert_payment_metadata_and_emit;

use super::{StableBalance, StableBalanceCore, per_receive_transfer_id};

/// How a per-receive conversion settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PerReceiveOutcome {
    /// The conversion ran and settled on this instance.
    Converted,
    /// The sent leg's record shows the swap ran, here or on another instance.
    AlreadyConverted,
    /// The sent leg's record shows the swap did not go through.
    ConversionFailed,
    /// Neither the sent leg's record nor the pool says yet how the swap ended.
    Undetermined,
    /// The payment no longer qualifies for conversion, for example because
    /// Stable Balance is off or the amount is below the minimum.
    Declined,
}

impl PerReceiveOutcome {
    /// The status the received payment ends with, or `None` while the outcome
    /// is unknown.
    pub(super) fn terminal_status(self) -> Option<ConversionStatus> {
        match self {
            Self::Converted | Self::AlreadyConverted => Some(ConversionStatus::Completed),
            Self::ConversionFailed | Self::Declined => Some(ConversionStatus::Failed),
            Self::Undetermined => None,
        }
    }

    pub(super) fn converted(self) -> bool {
        self == Self::Converted
    }
}

/// A swap that ran and then failed delivered the conversion, so it settles as
/// converted rather than as a failure. Returns its legs when both ids
/// resolved, and `None` when the swap ran but they did not.
fn settle_swap_that_ran(
    result: Result<TokenConversionResponse, ConversionError>,
) -> Result<Option<TokenConversionResponse>, ConversionError> {
    match result {
        Ok(response) => Ok(Some(response)),
        Err(ConversionError::FailedAfterSwap {
            message,
            sent_payment_id,
            received_payment_id,
        }) => {
            warn!("Conversion ran, then failed: {message}");
            Ok(sent_payment_id.zip(received_payment_id).map(
                |(sent_payment_id, received_payment_id)| TokenConversionResponse {
                    sent_payment_id,
                    received_payment_id,
                },
            ))
        }
        Err(e) => Err(e),
    }
}

/// How a swap ended, from the conversion info on one of its legs.
fn classify_per_receive_outcome(leg: &Payment) -> PerReceiveOutcome {
    let status = extract_conversion_info(leg.details.clone()).map(|info| info.status().clone());
    match status {
        Some(ConversionStatus::Completed) => PerReceiveOutcome::AlreadyConverted,
        Some(ConversionStatus::Failed | ConversionStatus::Refunded) => {
            PerReceiveOutcome::ConversionFailed
        }
        // A pending refund can still find the swap ran and mark the sent leg
        // Completed, so the outcome is not known until the refund settles.
        Some(ConversionStatus::Pending | ConversionStatus::RefundNeeded) | None => {
            PerReceiveOutcome::Undetermined
        }
    }
}

impl StableBalanceCore {
    /// How an earlier attempt ended, when its sent leg is stored. The
    /// deterministic transfer id names the sent leg, the sats sent to the
    /// pool, so a stored sent leg shows an attempt was made, not that the swap
    /// ran.
    pub(super) async fn sent_leg_outcome(
        &self,
        parent_payment_id: &str,
    ) -> Option<PerReceiveOutcome> {
        let transfer_id = per_receive_transfer_id(parent_payment_id);
        let sent_leg = self
            .storage
            .get_payment_by_id(transfer_id.to_string())
            .await
            .ok()?;
        Some(classify_per_receive_outcome(&sent_leg))
    }

    /// Asks the pool whether the swap ran when the sent leg's record is silent,
    /// linking its legs when it did. Errors when the pool could not be asked.
    pub(super) async fn find_completed_per_receive(
        &self,
        parent_payment_id: &str,
    ) -> Result<PerReceiveOutcome, ConversionError> {
        let transfer_id = per_receive_transfer_id(parent_payment_id);
        let found = self
            .token_converter
            .find_completed_conversion(&transfer_id, &ConversionPurpose::AutoConversion)
            .await?;
        let Some(response) = found else {
            return Ok(PerReceiveOutcome::Undetermined);
        };
        self.link_legs(parent_payment_id, &response).await?;
        Ok(PerReceiveOutcome::AlreadyConverted)
    }

    /// Links both legs of a conversion to the received payment it converted.
    pub(super) async fn link_legs(
        &self,
        parent_payment_id: &str,
        response: &TokenConversionResponse,
    ) -> Result<(), ConversionError> {
        for leg in [&response.sent_payment_id, &response.received_payment_id] {
            self.storage
                .insert_payment_metadata(
                    leg.clone(),
                    PaymentMetadata {
                        parent_payment_id: Some(parent_payment_id.to_string()),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Ok(())
    }

    /// Settles a task past the timeout from what is known of its swap, without
    /// converting. An outcome still unknown settles `Failed`, including a swap
    /// the pool does not report yet. Returns false, leaving the task to be
    /// tried again, when the status could not be written, or when the pool
    /// could not be asked and the lookup deadline has not passed.
    pub(super) async fn settle_timed_out(
        &self,
        event_emitter: &EventEmitter,
        parent_payment_id: &str,
    ) -> bool {
        let outcome = match self.sent_leg_outcome(parent_payment_id).await {
            Some(PerReceiveOutcome::Undetermined) | None => {
                match self.find_completed_per_receive(parent_payment_id).await {
                    Ok(outcome) => outcome,
                    Err(e) => {
                        if !self.queue.is_past_lookup_deadline(parent_payment_id).await {
                            warn!("Could not ask the pool about {parent_payment_id}: {e:?}");
                            return false;
                        }
                        warn!("Could not ask the pool about {parent_payment_id}, giving up: {e:?}");
                        PerReceiveOutcome::Undetermined
                    }
                }
            }
            Some(outcome) => outcome,
        };
        let status = outcome
            .terminal_status()
            .unwrap_or(ConversionStatus::Failed);
        self.finalize_and_emit(event_emitter, parent_payment_id, status)
            .await
            .is_ok()
    }
}

impl StableBalance {
    /// Converts a single received payment if it meets the minimum threshold.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn per_receive_convert(
        &self,
        parent_payment_id: &str,
    ) -> Result<PerReceiveOutcome, ConversionError> {
        // Ahead of the checks below: a swap that already ran or failed must
        // not read as declined because the minimum rose since.
        if let Some(outcome) = self.core.sent_leg_outcome(parent_payment_id).await {
            debug!(
                "Per-receive conversion for {parent_payment_id} already sent its leg: {outcome:?}"
            );
            if outcome != PerReceiveOutcome::Undetermined {
                return Ok(outcome);
            }
            return Ok(self
                .core
                .find_completed_per_receive(parent_payment_id)
                .await
                .unwrap_or_else(|e| {
                    warn!("Could not ask the pool about {parent_payment_id}: {e:?}");
                    PerReceiveOutcome::Undetermined
                }));
        }

        // Get the active token, skip if stable balance is inactive
        let Some(active_token_identifier) = self.core.get_active_token_identifier().await else {
            debug!("Per-receive conversion skipped: stable balance is inactive");
            return Ok(PerReceiveOutcome::Declined);
        };

        // Fetch payment from storage to get latest metadata and amount
        let payment = self
            .core
            .storage
            .get_payment_by_id(parent_payment_id.to_string())
            .await?;

        // A conversion receive is not converted again. It settles with how its
        // own swap ended.
        if let Some(PaymentDetails::Spark {
            conversion_info: Some(_),
            ..
        }) = &payment.details
        {
            debug!(
                "Per-receive conversion skipped: {} is a conversion receive",
                parent_payment_id
            );
            return Ok(match classify_per_receive_outcome(&payment) {
                PerReceiveOutcome::AlreadyConverted => PerReceiveOutcome::AlreadyConverted,
                _ => PerReceiveOutcome::Declined,
            });
        }

        // Check minimum threshold
        let amount_sats = payment.amount;
        let (_, min_from_amount) = self
            .core
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
        let amount_sats_u64 = u64::try_from(amount_sats).unwrap_or(u64::MAX);
        if amount_sats_u64 < min_from_amount {
            debug!("Per-receive conversion skipped: amount {amount_sats} < min {min_from_amount}");
            return Ok(PerReceiveOutcome::Declined);
        }

        // Generate deterministic transfer ID for idempotency
        let transfer_id = per_receive_transfer_id(parent_payment_id);
        debug!(
            "Per-receive deterministic id: {transfer_id} for payment id: {}",
            parent_payment_id
        );

        info!(
            "Per-receive conversion triggered: converting {amount_sats} sats to {active_token_identifier} for payment {parent_payment_id}",
        );

        // Perform conversion with deterministic transfer_id for idempotency
        let options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: self.core.config.max_slippage_bps,
            completion_timeout_secs: None,
        };
        let converted = self
            .core
            .token_converter
            .convert(
                self.event_emitter.clone(),
                &options,
                &ConversionPurpose::AutoConversion,
                Some(&active_token_identifier),
                ConversionAmount::AmountIn(amount_sats),
                Some(transfer_id),
            )
            .await;
        let Some(response) = settle_swap_that_ran(converted)? else {
            return Ok(PerReceiveOutcome::Converted);
        };
        self.core.link_legs(parent_payment_id, &response).await?;

        info!(
            "Per-receive conversion completed: converted {amount_sats} sats for {parent_payment_id} (sent={}, received={})",
            response.sent_payment_id, response.received_payment_id
        );

        Ok(PerReceiveOutcome::Converted)
    }

    /// Executes auto-conversion if the balance exceeds the threshold.
    ///
    /// Skips if:
    /// - A send-with-conversion payment is in flight (payment guard held)
    /// - Stable balance is inactive
    /// - Balance is below the trigger amount
    pub(super) async fn auto_convert(&self) -> Result<bool, ConversionError> {
        // Get the active token, skip if stable balance is inactive
        let Some(active_token_identifier) = self.core.get_active_token_identifier().await else {
            debug!("Auto-conversion skipped: stable balance is inactive");
            return Ok(false);
        };

        // Lock to atomically check "no payments in flight" + read balance.
        // This prevents a payment from starting between the check and the read,
        // which could inflate the balance with in-flight conversion funds.
        // The lock is released after reading — the captured balance is a clean snapshot.
        let balance_sats = {
            let _lock = self.payment_lock.lock().await;
            if self.payment_counter.load(Ordering::Relaxed) > 0 {
                debug!("Auto-conversion skipped: payments in flight");
                return Ok(false);
            }
            self.spark_wallet.get_balance().await?
        };

        // Check if balance exceeds the threshold
        let (threshold, _) = self
            .core
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
        if balance_sats < threshold {
            debug!("Auto-conversion skipped: balance {balance_sats} < threshold {threshold}");
            return Ok(false);
        }

        let from_btc_options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: self.core.config.max_slippage_bps,
            completion_timeout_secs: None,
        };

        // Check that converting wouldn't create token dust (balance below the ToBitcoin
        // min conversion limit, making it impossible to convert back).
        if self
            .produces_token_dust(&active_token_identifier, &from_btc_options, balance_sats)
            .await
        {
            return Ok(false);
        }

        // Yield to per-receive tasks that arrived while we were preparing.
        // Per-receive converts specific payment amounts and takes priority; if we
        // proceed, we'd convert the same sats and per-receive would fail with
        // InsufficientFunds. The next Synced event will re-queue auto-convert.
        if self.core.queue.has_per_receive().await {
            debug!("Auto-conversion aborted: per-receive tasks queued during preparation");
            return Ok(false);
        }

        info!(
            "Auto-conversion triggered: converting {balance_sats} sats to {active_token_identifier}",
        );

        let converted = self
            .core
            .token_converter
            .convert(
                self.event_emitter.clone(),
                &from_btc_options,
                &ConversionPurpose::AutoConversion,
                Some(&active_token_identifier),
                ConversionAmount::AmountIn(u128::from(balance_sats)),
                None,
            )
            .await;
        let Some(response) = settle_swap_that_ran(converted)? else {
            return Ok(true);
        };

        // Link sent payment as child of received payment
        self.core
            .storage
            .insert_payment_metadata(
                response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(response.received_payment_id.clone()),
                    ..Default::default()
                },
            )
            .await?;

        info!(
            "Auto-conversion completed: converted {} sats (sent_payment_id={}, received_payment_id={})",
            balance_sats, response.sent_payment_id, response.received_payment_id
        );

        // Persist Completed status for the received token payment
        insert_payment_metadata_and_emit(
            &self.core.storage,
            &self.event_emitter,
            response.received_payment_id.clone(),
            PaymentMetadata {
                conversion_status: Some(ConversionStatus::Completed),
                ..Default::default()
            },
        )
        .await?;

        Ok(true)
    }

    /// Converts the full token balance back to BTC on deactivation.
    ///
    /// Called by the conversion worker when stable balance is being deactivated.
    /// Converts all tokens of the given type back to Bitcoin. Skips if token
    /// balance is zero or below the minimum conversion limit.
    pub(super) async fn deactivation_convert(
        &self,
        token_identifier: &str,
    ) -> Result<bool, ConversionError> {
        // A recovered task can name the token that is active again, in which
        // case the deactivation it belongs to was cancelled.
        if self.core.get_active_token_identifier().await.as_deref() == Some(token_identifier) {
            debug!("Deactivation conversion skipped: {token_identifier} is active again");
            return Ok(false);
        }

        // Get the current token balance
        let token_balances = self.spark_wallet.get_token_balances().await?;
        let token_balance = token_balances
            .get(token_identifier)
            .map_or(0, |b| b.balance);

        if token_balance == 0 {
            debug!("Deactivation conversion skipped: zero token balance");
            return Ok(false);
        }

        // Check minimum conversion limit for ToBitcoin
        let limits = self
            .core
            .token_converter
            .fetch_limits(&FetchConversionLimitsRequest {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: token_identifier.to_string(),
                },
                token_identifier: Some(token_identifier.to_string()),
            })
            .await?;

        if let Some(min_from) = limits.min_from_amount
            && token_balance < min_from
        {
            debug!(
                "Deactivation conversion skipped: token balance {token_balance} < min {min_from}"
            );
            return Ok(false);
        }

        let to_btc_options = ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: token_identifier.to_string(),
            },
            max_slippage_bps: self.core.config.max_slippage_bps,
            completion_timeout_secs: None,
        };

        info!(
            "Deactivation conversion triggered: converting {token_balance} tokens ({token_identifier}) to BTC",
        );

        let converted = self
            .core
            .token_converter
            .convert(
                self.event_emitter.clone(),
                &to_btc_options,
                &ConversionPurpose::AutoConversion,
                Some(&token_identifier.to_string()),
                ConversionAmount::AmountIn(token_balance),
                None,
            )
            .await;
        let Some(response) = settle_swap_that_ran(converted)? else {
            return Ok(true);
        };

        // Link sent payment as child of received payment (same pattern as auto_convert)
        self.core
            .storage
            .insert_payment_metadata(
                response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(response.received_payment_id.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Persist Completed status for the received BTC payment
        insert_payment_metadata_and_emit(
            &self.core.storage,
            &self.event_emitter,
            response.received_payment_id.clone(),
            PaymentMetadata {
                conversion_status: Some(ConversionStatus::Completed),
                ..Default::default()
            },
        )
        .await?;

        info!(
            "Deactivation conversion completed: converted {token_balance} tokens (sent={}, received={})",
            response.sent_payment_id, response.received_payment_id
        );

        Ok(true)
    }

    /// Checks whether auto-converting `balance_sats` would create token dust
    /// (a balance below the `ToBitcoin` min conversion limit).
    async fn produces_token_dust(
        &self,
        active_token_identifier: &str,
        from_btc_options: &ConversionOptions,
        balance_sats: u64,
    ) -> bool {
        let token_id = active_token_identifier.to_string();

        // Fetch limits and token balances concurrently
        let limits_request = FetchConversionLimitsRequest {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: token_id.clone(),
            },
            token_identifier: Some(token_id.clone()),
        };
        let (limits_res, balances_res) = tokio::join!(
            self.core.token_converter.fetch_limits(&limits_request),
            self.spark_wallet.get_token_balances(),
        );

        let Some(to_btc_min) = limits_res.ok().and_then(|l| l.min_from_amount) else {
            return false;
        };

        let existing_tokens = balances_res
            .unwrap_or_default()
            .get(active_token_identifier)
            .map_or(0, |b| b.balance);

        if existing_tokens >= to_btc_min {
            return false;
        }

        // Estimate how many tokens we'd get from converting balance_sats
        let Ok(Some(est)) = self
            .core
            .token_converter
            .validate(
                Some(from_btc_options),
                Some(&token_id),
                ConversionAmount::AmountIn(u128::from(balance_sats)),
            )
            .await
        else {
            return false;
        };

        // Would create token dust if projected balance is still below min conversion limit
        let estimated_total = existing_tokens.saturating_add(est.amount_out);
        if estimated_total < to_btc_min {
            debug!(
                "Auto-conversion skipped: {balance_sats} sats would produce \
                 {} tokens, total {estimated_total} still below ToBitcoin min {to_btc_min} \
                 (existing tokens: {existing_tokens})",
                est.amount_out,
            );
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversionInfo, PaymentMethod, PaymentStatus, PaymentType};

    fn sent_leg(status: Option<ConversionStatus>) -> Payment {
        Payment {
            id: "sent-leg".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 5_000,
            fees: 0,
            timestamp: 1,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: status.map(|status| ConversionInfo::Amm {
                    pool_id: "pool".to_string(),
                    conversion_id: "conversion".to_string(),
                    status,
                    fee: None,
                    purpose: None,
                    amount_adjustment: None,
                    degradation: None,
                }),
            }),
            conversion_details: None,
        }
    }

    /// The sent leg's own record decides the outcome.
    #[test]
    fn a_sent_leg_is_classified_by_how_its_swap_ended() {
        let cases = [
            (
                Some(ConversionStatus::Completed),
                PerReceiveOutcome::AlreadyConverted,
            ),
            (
                Some(ConversionStatus::Refunded),
                PerReceiveOutcome::ConversionFailed,
            ),
            (
                Some(ConversionStatus::RefundNeeded),
                PerReceiveOutcome::Undetermined,
            ),
            (
                Some(ConversionStatus::Failed),
                PerReceiveOutcome::ConversionFailed,
            ),
            (
                Some(ConversionStatus::Pending),
                PerReceiveOutcome::Undetermined,
            ),
            (None, PerReceiveOutcome::Undetermined),
        ];
        for (status, expected) in cases {
            assert_eq!(
                classify_per_receive_outcome(&sent_leg(status.clone())),
                expected,
                "{status:?}"
            );
        }
    }

    /// Only an unknown outcome leaves the payment unsettled.
    #[test]
    fn each_outcome_settles_to_its_status() {
        use PerReceiveOutcome::*;
        let cases = [
            (Converted, Some(ConversionStatus::Completed), true),
            (AlreadyConverted, Some(ConversionStatus::Completed), false),
            (ConversionFailed, Some(ConversionStatus::Failed), false),
            (Declined, Some(ConversionStatus::Failed), false),
            (Undetermined, None, false),
        ];
        for (outcome, status, converted) in cases {
            assert_eq!(outcome.terminal_status(), status, "{outcome:?}");
            assert_eq!(outcome.converted(), converted, "{outcome:?}");
        }
    }

    fn failed_after_swap(sent: Option<&str>, received: Option<&str>) -> ConversionError {
        ConversionError::FailedAfterSwap {
            message: "failed after the swap".to_string(),
            sent_payment_id: sent.map(str::to_string),
            received_payment_id: received.map(str::to_string),
        }
    }

    /// A swap that ran counts as converted, and its legs are returned only when
    /// both ids resolved.
    #[test]
    fn a_swap_that_ran_settles_with_its_legs() {
        let legs = settle_swap_that_ran(Err(failed_after_swap(Some("sent"), Some("received"))))
            .unwrap()
            .unwrap();
        assert_eq!(legs.sent_payment_id, "sent");
        assert_eq!(legs.received_payment_id, "received");

        for (sent, received) in [(Some("sent"), None), (None, Some("received")), (None, None)] {
            assert!(
                settle_swap_that_ran(Err(failed_after_swap(sent, received)))
                    .unwrap()
                    .is_none(),
                "{sent:?} {received:?}"
            );
        }

        let failed = settle_swap_that_ran(Err(ConversionError::ConversionFailed("x".to_string())));
        assert!(matches!(failed, Err(ConversionError::ConversionFailed(_))));

        let converted = settle_swap_that_ran(Ok(TokenConversionResponse {
            sent_payment_id: "sent".to_string(),
            received_payment_id: "received".to_string(),
        }))
        .unwrap()
        .unwrap();
        assert_eq!(converted.received_payment_id, "received");
    }
}
