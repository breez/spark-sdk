use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use spark_wallet::SparkWallet;
use tokio::sync::{Notify, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info, warn};

use crate::{
    models::StableBalanceConfig,
    token_conversion::{
        ConversionAmount, ConversionError, ConversionOptions, ConversionPurpose, ConversionType,
        FetchConversionLimitsRequest, TokenConverter,
    },
    utils::expiring_cell::ExpiringCell,
};

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

    /// Counter of active conversions.
    /// Auto-convert waits while this is > 0 and is notified when `active_conversions` drops to 0
    active_conversions: Arc<AtomicUsize>,
    conversions_done: Arc<Notify>,

    /// Notify to trigger auto-conversion
    auto_convert_trigger: Arc<Notify>,
}

/// Guard that tracks an active conversions.
///
/// When dropped, decrements the active conversions counter and
/// notifies waiting auto-convert tasks if the counter reaches zero.
/// Creating this guard is non-blocking - conversions never wait.
pub(crate) struct ActiveConversionGuard {
    active_counter: Arc<AtomicUsize>,
    done_notify: Arc<Notify>,
}

impl Drop for ActiveConversionGuard {
    fn drop(&mut self) {
        let prev = self.active_counter.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Was 1, now 0 - notify waiting auto-convert
            self.done_notify.notify_waiters();
        }
    }
}

impl StableBalance {
    /// Creates a new `StableBalance` instance and spawns the auto-convert background task.
    pub fn new(
        config: StableBalanceConfig,
        token_converter: Arc<dyn TokenConverter>,
        spark_wallet: Arc<SparkWallet>,
        shutdown_receiver: watch::Receiver<()>,
    ) -> Self {
        let active_conversions = Arc::new(AtomicUsize::new(0));
        let conversions_done = Arc::new(Notify::new());
        let auto_convert_trigger = Arc::new(Notify::new());

        let stable_balance = Self {
            config,
            token_converter,
            spark_wallet,
            effective_values: Arc::new(ExpiringCell::new()),
            active_conversions,
            conversions_done,
            auto_convert_trigger,
        };

        // Spawn the background auto-convert task
        stable_balance.spawn_auto_convert_task(shutdown_receiver);

        stable_balance
    }

    /// Spawns the background task that handles auto-conversion triggers.
    ///
    /// The task:
    /// 1. Waits for a trigger signal
    /// 2. Waits for all active conversions to complete
    /// 3. Executes auto-conversion if conditions are met
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
                    () = stable_balance.auto_convert_trigger.notified() => {
                        debug!("Auto-conversion triggered");
                    }
                }

                // Wait for active conversions to complete
                while stable_balance.active_conversions.load(Ordering::SeqCst) > 0 {
                    debug!("Auto-conversion waiting for active conversions to complete");
                    stable_balance.conversions_done.notified().await;
                }

                // Execute auto-conversion
                if let Err(e) = stable_balance.auto_convert().await {
                    warn!("Auto-conversion failed: {e:?}");
                }
            }
        });
    }

    /// Executes auto-conversion if the balance exceeds the threshold.
    async fn auto_convert(&self) -> Result<bool, ConversionError> {
        // Get or initialize effective threshold and reserved values
        let (threshold, reserved) = self.get_or_init_effective_values().await?;

        // Get current balance
        let balance_sats = self.spark_wallet.get_balance().await?;

        // Skip if balance is less than reserved + threshold
        let trigger_amount = reserved.saturating_add(threshold);
        if balance_sats < trigger_amount {
            debug!(
                "Auto-conversion skipped: balance {} < reserved {} + threshold {}",
                balance_sats, reserved, threshold
            );
            return Ok(false);
        }

        // Convert only the amount above the reserve
        let amount_to_convert = balance_sats.saturating_sub(reserved);

        info!(
            "Auto-conversion triggered: converting {} sats to {} (keeping {} sats reserved)",
            amount_to_convert, self.config.token_identifier, reserved
        );

        let options = ConversionOptions {
            conversion_type: ConversionType::FromBitcoin,
            max_slippage_bps: self.config.max_slippage_bps,
            completion_timeout_secs: None,
        };

        let _response = self
            .token_converter
            .convert(
                &options,
                &ConversionPurpose::AutoConversion,
                Some(&self.config.token_identifier),
                ConversionAmount::AmountIn(u128::from(amount_to_convert)),
            )
            .await?;
        info!("Auto-conversion completed");

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
        debug!("Triggering auto-conversion");
        self.auto_convert_trigger.notify_one();
    }

    /// Signals the start of a payment conversion.
    ///
    /// Returns a guard that must be held for the duration of the payment conversion.
    /// When the guard is dropped, it decrements the active counter and notifies
    /// any waiting auto-convert task.
    ///
    /// This operation is non-blocking - payment conversions never wait.
    pub fn signal_active_conversion(&self) -> ActiveConversionGuard {
        self.active_conversions.fetch_add(1, Ordering::SeqCst);
        ActiveConversionGuard {
            active_counter: Arc::clone(&self.active_conversions),
            done_notify: Arc::clone(&self.conversions_done),
        }
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

        let balance_sats = self.spark_wallet.get_balance().await?;

        // Only auto-populate if there's not enough sats balance
        if u128::from(balance_sats) >= payment_amount {
            return Ok(None);
        }

        info!(
            "Auto-populating conversion options: balance {balance_sats} sats < payment amount {payment_amount} sats"
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
