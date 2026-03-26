//! Conversion logic for stable balance.
//!
//! Contains the actual conversion methods called by the unified worker:
//! - `per_receive_convert`: converts individual received payments
//! - `auto_convert`: batch converts accumulated BTC above threshold
//! - `deactivation_convert`: converts all tokens back to BTC on deactivation

use tracing::{debug, info};

use crate::models::{ConversionStatus, PaymentDetails};
use crate::persist::PaymentMetadata;
use crate::token_conversion::{
    ConversionAmount, ConversionError, ConversionOptions, ConversionPurpose, ConversionType,
    FetchConversionLimitsRequest,
};

use super::{StableBalance, per_receive_transfer_id};

/// How long the BTC balance must remain unchanged before auto-converting.
const AUTO_CONVERT_DEBOUNCE_SECS: u64 = 60;

/// Result of a debounced auto-convert attempt.
pub(super) enum AutoConvertResult {
    /// Conversion executed (may or may not have converted).
    Done { converted: bool },
    /// Debounce timer not elapsed — task should stay in queue.
    Debounced,
}

impl StableBalance {
    /// Converts a single received payment if it meets the minimum threshold.
    ///
    /// Returns `true` if conversion was performed, `false` if skipped.
    /// Acquires a payment lock guard to block auto-convert on other instances.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn per_receive_convert(
        &self,
        parent_payment_id: &str,
    ) -> Result<bool, ConversionError> {
        // Get the active token, skip if stable balance is inactive
        let Some(active_token_identifier) = self.get_active_token_identifier().await else {
            debug!("Per-receive conversion skipped: stable balance is inactive");
            return Ok(false);
        };

        // Fetch payment from storage to get latest metadata and amount
        let payment = self
            .storage
            .get_payment_by_id(parent_payment_id.to_string())
            .await?;

        // Skip if this spark payment has conversion info (it's a conversion receive itself)
        if let Some(PaymentDetails::Spark {
            conversion_info: Some(_),
            ..
        }) = &payment.details
        {
            debug!(
                "Per-receive conversion skipped: {} is a conversion receive",
                parent_payment_id
            );
            return Ok(false);
        }

        // Check minimum threshold
        let amount_sats = payment.amount;
        let (_, min_from_amount) = self
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
        let amount_sats_u64 = u64::try_from(amount_sats).unwrap_or(u64::MAX);
        if amount_sats_u64 < min_from_amount {
            debug!("Per-receive conversion skipped: amount {amount_sats} < min {min_from_amount}");
            return Ok(false);
        }

        // Generate deterministic transfer ID for idempotency
        let transfer_id = per_receive_transfer_id(parent_payment_id);
        debug!(
            "Per-receive deterministic id: {transfer_id} for payment id: {}",
            parent_payment_id
        );

        // Check if payment with this transfer_id already exists (already converted)
        if self
            .storage
            .get_payment_by_id(transfer_id.to_string())
            .await
            .is_ok()
        {
            debug!(
                "Per-receive conversion skipped: payment {} already exists",
                transfer_id
            );
            return Ok(false);
        }

        info!(
            "Per-receive conversion triggered: converting {amount_sats} sats to {active_token_identifier} for payment {parent_payment_id}",
        );

        // Perform conversion with deterministic transfer_id for idempotency
        let options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: self.config.max_slippage_bps,
            completion_timeout_secs: None,
        };
        let response = self
            .token_converter
            .convert(
                &options,
                &ConversionPurpose::AutoConversion,
                Some(&active_token_identifier),
                ConversionAmount::AmountIn(amount_sats),
                Some(transfer_id),
            )
            .await?;

        // Link both conversion payments to the received parent payment
        self.storage
            .insert_payment_metadata(
                response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(parent_payment_id.to_string()),
                    ..Default::default()
                },
            )
            .await?;
        self.storage
            .insert_payment_metadata(
                response.received_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(parent_payment_id.to_string()),
                    ..Default::default()
                },
            )
            .await?;

        info!(
            "Per-receive conversion completed: converted {amount_sats} sats for {parent_payment_id} (sent={}, received={})",
            response.sent_payment_id, response.received_payment_id
        );

        Ok(true)
    }

    /// Executes auto-conversion if the balance exceeds the threshold.
    ///
    /// Skips if:
    /// - Stable balance is inactive
    /// - Balance is below the trigger amount
    pub(super) async fn auto_convert(&self) -> Result<bool, ConversionError> {
        // Get the active token, skip if stable balance is inactive
        let Some(active_token_identifier) = self.get_active_token_identifier().await else {
            debug!("Auto-conversion skipped: stable balance is inactive");
            return Ok(false);
        };

        // Check if balance exceeds the threshold
        let (threshold, _) = self
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
        let balance_sats = self.spark_wallet.get_balance().await?;
        if balance_sats < threshold {
            debug!("Auto-conversion skipped: balance {balance_sats} < threshold {threshold}");
            return Ok(false);
        }

        let from_btc_options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: self.config.max_slippage_bps,
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

        info!(
            "Auto-conversion triggered: converting {balance_sats} sats to {active_token_identifier}",
        );

        let response = self
            .token_converter
            .convert(
                &from_btc_options,
                &ConversionPurpose::AutoConversion,
                Some(&active_token_identifier),
                ConversionAmount::AmountIn(u128::from(balance_sats)),
                None,
            )
            .await?;

        // Link sent payment as child of received payment
        self.storage
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
        self.storage
            .insert_payment_metadata(
                response.received_payment_id.clone(),
                PaymentMetadata {
                    conversion_status: Some(ConversionStatus::Completed),
                    ..Default::default()
                },
            )
            .await?;

        Ok(true)
    }

    /// Auto-convert with debounce ([`AUTO_CONVERT_DEBOUNCE_SECS`] secs).
    ///
    /// Checks the balance snapshot to determine if the balance has been stable
    /// for the debounce period. If the balance changed, updates the snapshot
    /// and returns [`AutoConvertResult::Debounced`]. If stable long enough,
    /// proceeds with conversion.
    pub(super) async fn debounced_auto_convert(
        &self,
    ) -> Result<AutoConvertResult, ConversionError> {
        let current_balance = self.spark_wallet.get_balance().await?;
        let now = super::queue::now_secs();

        {
            let mut snapshot = self.balance_snapshot.lock().await;
            match snapshot.as_mut() {
                // No snapshot — first check or label change, skip debounce
                None => {
                    *snapshot = Some(super::BalanceSnapshot {
                        balance: current_balance,
                        updated_at: now,
                    });
                    debug!("Auto-convert debounce: skipped (no snapshot)");
                    let converted = self.auto_convert().await?;
                    return Ok(AutoConvertResult::Done { converted });
                }
                Some(s) if s.balance != current_balance => {
                    debug!(
                        "Auto-convert debounce: balance changed to {current_balance} sats, resetting timer"
                    );
                    s.balance = current_balance;
                    s.updated_at = now;
                    return Ok(AutoConvertResult::Debounced);
                }
                Some(s) => {
                    let elapsed = now.saturating_sub(s.updated_at);
                    if elapsed < AUTO_CONVERT_DEBOUNCE_SECS {
                        debug!(
                            "Auto-convert debounce: {elapsed}s elapsed, need {AUTO_CONVERT_DEBOUNCE_SECS}s of stability"
                        );
                        return Ok(AutoConvertResult::Debounced);
                    }
                }
            }
        }

        debug!(
            "Auto-convert debounce: balance stable at {current_balance} sats for ≥{AUTO_CONVERT_DEBOUNCE_SECS}s, proceeding"
        );
        let converted = self.auto_convert().await?;
        Ok(AutoConvertResult::Done { converted })
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
            max_slippage_bps: self.config.max_slippage_bps,
            completion_timeout_secs: None,
        };

        info!(
            "Deactivation conversion triggered: converting {token_balance} tokens ({token_identifier}) to BTC",
        );

        let response = self
            .token_converter
            .convert(
                &to_btc_options,
                &ConversionPurpose::AutoConversion,
                Some(&token_identifier.to_string()),
                ConversionAmount::AmountIn(token_balance),
                None,
            )
            .await?;

        // Link sent payment as child of received payment (same pattern as auto_convert)
        self.storage
            .insert_payment_metadata(
                response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(response.received_payment_id.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Persist Completed status for the received BTC payment
        self.storage
            .insert_payment_metadata(
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
            self.token_converter.fetch_limits(&limits_request),
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
        let estimated_total = existing_tokens.saturating_add(est.amount);
        if estimated_total < to_btc_min {
            debug!(
                "Auto-conversion skipped: {balance_sats} sats would produce \
                 {} tokens, total {estimated_total} still below ToBitcoin min {to_btc_min} \
                 (existing tokens: {existing_tokens})",
                est.amount,
            );
            return true;
        }

        false
    }
}
