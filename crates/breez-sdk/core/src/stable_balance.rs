use std::sync::Arc;

use spark_wallet::SparkWallet;
use tokio::sync::{Notify, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info, warn};

use breez_sdk_common::sync::SigningClient;

use crate::realtime_sync::sync_lock::{LockCounter, SyncLockGuard};

use crate::{
    models::StableBalanceConfig,
    token_conversion::{
        ConversionAmount, ConversionError, ConversionOptions, ConversionPurpose, ConversionType,
        FetchConversionLimitsRequest, TokenConverter,
    },
    utils::expiring_cell::ExpiringCell,
};

/// Lock name for payment conversion guards (non-exclusive, fire-and-forget).
const PAYMENT_LOCK_NAME: &str = "payments_conversion";
/// Lock name for auto-conversion (exclusive).
const AUTO_CONVERT_LOCK_NAME: &str = "auto_conversion";
/// TTL for cached effective values (1 hour)
const EFFECTIVE_VALUES_TTL_MS: u128 = 3_600_000;

/// Cached effective threshold and reserved values for auto-conversion.
#[derive(Clone)]
struct EffectiveValues {
    threshold: u64,
    reserved: u64,
}

/// Manages stable balance auto-conversion behavior.
///
/// This struct handles the business logic of when and how much to convert,
/// while delegating the actual conversion mechanics to a `TokenConverter`.
/// It coordinates with payment conversion flows to prevent race conditions.
#[derive(Clone)]
pub(crate) struct StableBalance {
    /// Configuration for stable balance behavior
    config: StableBalanceConfig,

    /// Reference to the token converter for executing conversions
    token_converter: Arc<dyn TokenConverter>,

    /// Reference to the spark wallet for balance queries
    spark_wallet: Arc<SparkWallet>,

    /// Cached effective values for auto-conversion (expires after TTL, shared across clones)
    effective_values: Arc<ExpiringCell<EffectiveValues>>,

    /// Tracks the number of in-flight payment conversions.
    /// Auto-convert is skipped while any payments are ongoing.
    ongoing_payments: Arc<LockCounter>,

    /// Notify to trigger auto-conversion
    auto_convert_trigger: Arc<Notify>,

    /// Optional signing client for coordinating across SDK instances.
    /// `None` when real-time sync is not configured.
    signing_client: Option<SigningClient>,
}

impl StableBalance {
    /// Creates a new `StableBalance` instance and spawns the auto-convert background task.
    pub fn new(
        config: StableBalanceConfig,
        token_converter: Arc<dyn TokenConverter>,
        spark_wallet: Arc<SparkWallet>,
        shutdown_receiver: watch::Receiver<()>,
        signing_client: Option<SigningClient>,
    ) -> Self {
        let auto_convert_trigger = Arc::new(Notify::new());

        let stable_balance = Self {
            config,
            token_converter,
            spark_wallet,
            effective_values: Arc::new(ExpiringCell::new()),
            ongoing_payments: Arc::new(LockCounter::new()),
            auto_convert_trigger,
            signing_client,
        };

        // Spawn the background auto-convert task
        stable_balance.spawn_auto_convert_task(shutdown_receiver);

        stable_balance
    }

    /// Spawns the background task that handles auto-conversion triggers.
    ///
    /// The task:
    /// 1. Waits for a trigger signal
    /// 2. Executes auto-conversion if conditions are met
    fn spawn_auto_convert_task(&self, mut shutdown_receiver: watch::Receiver<()>) {
        let stable_balance = self.clone();

        tokio::spawn(async move {
            loop {
                // Wait for a trigger or shutdown
                tokio::select! {
                    _ = shutdown_receiver.changed() => {
                        info!("Auto-conversion task shutdown signal received");
                        return;
                    }
                    () = stable_balance.auto_convert_trigger.notified() => {}
                }

                if let Err(e) = stable_balance.auto_convert().await {
                    warn!("Auto-conversion failed: {e:?}");
                }
            }
        });
    }

    /// Executes auto-conversion if the balance exceeds the threshold.
    async fn auto_convert(&self) -> Result<bool, ConversionError> {
        // 1. Check no payments are ongoing
        let ongoing = self.ongoing_payments.get();
        if ongoing > 0 {
            debug!("Auto-conversion skipped: {ongoing} payment(s) in progress");
            return Ok(false);
        }

        // 2. Check if balance exceeds the trigger amount
        let (threshold, reserved) = self.get_or_init_effective_values().await?;
        let balance_sats = self.spark_wallet.get_balance().await?;
        let trigger_amount = reserved.saturating_add(threshold);
        if balance_sats < trigger_amount {
            debug!(
                "Auto-conversion skipped: balance {balance_sats} < reserved {reserved} + threshold {threshold}"
            );
            return Ok(false);
        }

        // 3. Check if payment conversions are in progress on other instances
        if let Some(client) = &self.signing_client {
            match client.get_lock(PAYMENT_LOCK_NAME).await {
                Ok(true) => {
                    debug!("Auto-conversion skipped: payments lock held on another instance");
                    return Ok(false);
                }
                Ok(false) => {}
                Err(e) => {
                    debug!("Auto-conversion skipped: failed to check payments lock: {e:?}");
                    return Ok(false);
                }
            }
        }

        // 4. Acquire exclusive auto-conversion lock — skip if another instance holds it
        let _lock_guard = match SyncLockGuard::new_exclusive(
            AUTO_CONVERT_LOCK_NAME.to_string(),
            self.signing_client.clone(),
        )
        .await
        {
            Ok(guard) => guard,
            Err(e) => {
                debug!("Auto-conversion skipped: failed to acquire exclusive lock: {e:?}");
                return Ok(false);
            }
        };

        // 5. Convert the amount above the reserve
        let amount_to_convert = balance_sats.saturating_sub(reserved);

        info!(
            "Auto-conversion triggered: converting {amount_to_convert} sats to {} (keeping {reserved} sats reserved)",
            self.config.token_identifier,
        );

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
                Some(&self.config.token_identifier),
                ConversionAmount::AmountIn(u128::from(amount_to_convert)),
            )
            .await?;
        info!(
            "Auto-conversion completed: converted {} sats (sent_payment_id={}, received_payment_id={})",
            amount_to_convert, response.sent_payment_id, response.received_payment_id
        );

        // _lock_guard drops here, releasing the distributed lock

        Ok(true)
    }

    /// Gets or initializes the effective threshold and reserved sats for auto-conversion.
    ///
    /// Returns cached values if they exist and haven't expired. Otherwise, fetches
    /// conversion limits and computes:
    /// - Effective threshold: `max(user_threshold, min_from_amount)`
    /// - Effective reserved: user value if set, otherwise `min_from_amount`
    ///
    /// Values are cached with a TTL and will be refreshed after expiration.
    async fn get_or_init_effective_values(&self) -> Result<(u64, u64), ConversionError> {
        // Return cached values if not expired
        if let Some(values) = self.effective_values.get().await {
            return Ok((values.threshold, values.reserved));
        }

        // Fetch limits and compute effective values
        let limits = self
            .token_converter
            .fetch_limits(&FetchConversionLimitsRequest {
                conversion_type: ConversionType::FromBitcoin,
                token_identifier: Some(self.config.token_identifier.clone()),
            })
            .await?;

        let min_from_amount =
            u64::try_from(limits.min_from_amount.unwrap_or(0)).unwrap_or(u64::MAX);

        // Compute effective threshold: max(user_threshold, min_from_amount)
        let threshold = match self.config.threshold_sats {
            Some(t) if t >= min_from_amount => t,
            Some(_) | None => min_from_amount,
        };

        // Compute effective reserved: user value if set, otherwise min_from_amount
        let reserved = self.config.reserved_sats.unwrap_or(min_from_amount);

        // Cache with TTL
        self.effective_values
            .set(
                EffectiveValues {
                    threshold,
                    reserved,
                },
                EFFECTIVE_VALUES_TTL_MS,
            )
            .await;
        info!(
            "Auto-conversion effective values initialized: threshold={threshold} sats, reserved={reserved} sats"
        );

        Ok((threshold, reserved))
    }

    /// Triggers the auto-conversion task.
    ///
    /// This is a non-blocking operation that sends a signal to the background task.
    /// The actual conversion will wait for any active conversions to complete.
    pub fn trigger_auto_convert(&self) {
        self.auto_convert_trigger.notify_one();
    }

    /// Creates a lock guard that prevents auto-conversion while held.
    ///
    /// Auto-convert is skipped while any guard is active. When the
    /// last guard is dropped, the distributed lock is released (if configured).
    pub fn create_payment_lock_guard(&self) -> SyncLockGuard {
        SyncLockGuard::new(
            PAYMENT_LOCK_NAME.to_string(),
            Arc::clone(&self.ongoing_payments),
            self.signing_client.clone(),
        )
    }

    /// Gets conversion options for a payment if auto-population is needed.
    ///
    /// Returns `Some(ConversionOptions)` if:
    /// - No explicit options were provided
    /// - The payment is not a token payment (`token_identifier` is None)
    /// - The current sats balance is insufficient for the payment amount
    ///
    /// In this case, returns options to convert from the configured stable token to Bitcoin.
    pub async fn get_conversion_options(
        &self,
        options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        payment_amount: u128,
    ) -> Result<Option<ConversionOptions>, ConversionError> {
        // Use provided options if explicitly set
        if options.is_some() {
            return Ok(options.cloned());
        }

        // Don't auto-convert for token payments
        if token_identifier.is_some() {
            return Ok(None);
        }

        let (_, reserved) = self.get_or_init_effective_values().await?;
        let balance_sats = self.spark_wallet.get_balance().await?;
        let effective_balance = balance_sats.min(reserved);

        // Only auto-populate if the effective sats balance (capped at reserve) is insufficient.
        // Sats above the reserve are expected to be used for payments or eventually
        // auto-converted to tokens, so they shouldn't be counted as available for
        // direct sats payments.
        if u128::from(effective_balance) >= payment_amount {
            return Ok(None);
        }

        info!(
            "Auto-populating conversion options: effective balance {effective_balance} sats \
             (balance={balance_sats}, reserve={reserved}) < payment amount {payment_amount} sats"
        );
        Ok(Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: self.config.token_identifier.clone(),
            },
            max_slippage_bps: self.config.max_slippage_bps,
            completion_timeout_secs: None,
        }))
    }
}
