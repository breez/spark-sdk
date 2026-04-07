//! Stable balance management for automatic BTC-to-token conversion.
//!
//! This module provides functionality to maintain a "stable balance" by automatically
//! converting received BTC to a configured stable token when thresholds are exceeded.
//! The active token can be changed at runtime via [`crate::models::UpdateUserSettingsRequest`].
//!
//! # High-Level Flow
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                              STARTUP                                │
//! │  1. Resolve active token (cache → config default → inactive)        │
//! │  2. Spawn conversion worker (waits for initial sync)                │
//! │  3. Pre-warm effective values cache (threshold, min limits)         │
//! │  4. Recover pending conversions from previous session               │
//! │  5. Queue cold-start auto-convert                                   │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                          EVENT MIDDLEWARE                           │
//! │                                                                     │
//! │  PaymentSucceeded ──┬─► Matches deferred transfer_id? ──► Resolve   │
//! │                     │                                               │
//! │                     ├─► Is receive + sats + ≥ min? ──► PerReceive   │
//! │                     │                                               │
//! │                     └─► Otherwise ──────────────────► AutoConvert   │
//! │                                                                     │
//! │  Synced ──────────────► Expire deferred tasks older than 120s       │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                          CONVERSION QUEUE                           │
//! │                                                                     │
//! │  Priority order:                                                    │
//! │    1. PerReceive(payment_id)  — convert individual received sats    │
//! │    2. Deactivation(token_id)  — convert active token back to BTC    │
//! │    3. AutoConvert             — batch convert excess BTC balance    │
//! │                                                                     │
//! │  Rules:                                                             │
//! │  • PerReceive deduplicates by payment_id                            │
//! │  • AutoConvert collapses multiple triggers into one                 │
//! │  • Deactivation overrides pending AutoConvert                       │
//! │  • AutoConvert/Deactivation only runs when no PerReceive pending    │
//! │    (including deferred — they may still need those sats)            │
//! │  • Deferred tasks are skipped until resolved or timed out           │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                  │
//!                                  ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    CONVERSION WORKER (serial)                       │
//! │                                                                     │
//! │    ┌─────────────────────────────────────────────────────────┐      │
//! │    │ PerReceive                                              │      │
//! │    │  • Check active token, payment lock, min amount         │      │
//! │    │  • Deterministic transfer_id for idempotency            │      │
//! │    │  • BTC → Token conversion (amount = payment amount)     │      │
//! │    │  • On failure → Defer (wait for other instance or       │      │
//! │    │    timeout after 120s)                                  │      │
//! │    │  • On success → mark Completed, trigger sync            │      │
//! │    └─────────────────────────────────────────────────────────┘      │
//! │                                                                     │
//! │    ┌─────────────────────────────────────────────────────────┐      │
//! │    │ AutoConvert                                             │      │
//! │    │  • Check active token, ongoing payments, balance        │      │
//! │    │  • Acquire exclusive auto_conversion lock               │      │
//! │    │  • Check for token dust (would balance be below         │      │
//! │    │    ToBitcoin min limit?)                                │      │
//! │    │  • BTC → Token conversion (amount = full BTC balance)   │      │
//! │    │  • On success → trigger sync                            │      │
//! │    └─────────────────────────────────────────────────────────┘      │
//! │                                                                     │
//! │    ┌─────────────────────────────────────────────────────────┐      │
//! │    │ Deactivation                                            │      │
//! │    │  • Get token balance, check min conversion limit        │      │
//! │    │  • Acquire exclusive auto_conversion lock               │      │
//! │    │  • Token → BTC conversion (amount = full token balance) │      │
//! │    │  • On success → trigger sync                            │      │
//! │    └─────────────────────────────────────────────────────────┘      │
//! │                                                                     │
//! └─────────────────────────────────────────────────────────────────────┘
//!
//! # Amount Adjustments
//!
//! Conversion amounts may be adjusted before execution to respect limits
//! and avoid creating unconvertible "token dust". Adjustments are tracked
//! via [`AmountAdjustmentReason`] in conversion metadata for UI visibility.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                       CONVERSION AMOUNTS                            │
//! │                                                                     │
//! │  AmountIn(sats)      — "convert exactly this much"                  │
//! │    Used by: PerReceive, AutoConvert, Deactivation                   │
//! │    Slippage applied to estimated output (conservative estimate)     │
//! │                                                                     │
//! │  MinAmountOut(sats)  — "I need at least this much out"              │
//! │    Used by: Send-with-conversion (Token → BTC for payments)         │
//! │    SDK calculates required input from the pool estimate             │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                  │
//!                                  ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                   ADJUSTMENTS (ToBitcoin only)                      │
//! │                                                                     │
//! │  1. Floor to minimum                                                │
//! │     amount_in < min_from_amount → increase to min_from_amount       │
//! │     Reason: FlooredToMinLimit                                       │
//! │                                                                     │
//! │  2. Dust avoidance                                                  │
//! │     remaining = token_balance - amount_in                           │
//! │     If 0 < remaining < min_from_amount →                            │
//! │       convert entire token_balance instead                          │
//! │     Reason: IncreasedToAvoidDust                                    │
//! │                                                                     │
//! │  FromBitcoin conversions: no adjustments (dust check is done        │
//! │  pre-flight via produces_token_dust() in AutoConvert)               │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Send-With-Conversion (outgoing payments)
//!
//! When sending BTC but sats balance is insufficient:
//! ```text
//! send_payment() → get_conversion_options()
//!   • If stable balance active + no explicit options + sats < amount
//!   • Auto-populates Token → BTC conversion options
//!   • PaymentGuard held for duration of send (suppresses auto-convert)
//! ```

mod conversions;
mod queue;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use platform_utils::tokio;
use spark_wallet::{SparkWallet, TransferId};
use tokio::sync::{Mutex, Notify, RwLock, watch};
use tracing::{debug, info, warn};

use self::queue::ConversionQueue;
pub(crate) use self::queue::PendingConversion;

use crate::events::{EventEmitter, EventMiddleware, SdkEvent};
use crate::models::{
    ConversionDetails, ConversionStatus, Payment, PaymentMethod, PaymentType, StableBalanceToken,
};
use crate::persist::{ObjectCacheRepository, PaymentMetadata, Storage};
use crate::{
    SdkError,
    models::StableBalanceConfig,
    sdk::SyncCoordinator,
    token_conversion::{
        ConversionError, ConversionOptions, ConversionType, FetchConversionLimitsRequest,
        TokenConverter,
    },
    utils::expiring_cell::ExpiringCell,
};

/// TTL for cached effective values (1 hour)
pub(super) const EFFECTIVE_VALUES_TTL_MS: u128 = 3_600_000;

/// Deterministic transfer ID for a per-receive conversion.
/// Used for idempotency across instances.
pub(super) fn per_receive_transfer_id(payment_id: &str) -> TransferId {
    TransferId::from_name(&format!("receive_conversion:{payment_id}"))
}

/// Cached effective threshold and min conversion limit for auto-conversion.
#[derive(Clone)]
pub(super) struct EffectiveValues {
    pub threshold: u64,
    pub min_from_amount: u64,
}

/// RAII guard that tracks an in-flight send-with-conversion payment.
///
/// While held, auto-convert is suppressed to avoid converting BTC that
/// is about to be spent. When dropped, decrements the counter and wakes
/// the conversion worker so it can re-evaluate.
pub(crate) struct PaymentGuard {
    counter: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl Drop for PaymentGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
        self.notify.notify_one();
    }
}

/// Manages stable balance auto-conversion behavior.
///
/// This struct handles the business logic of when and how much to convert,
/// while delegating the actual conversion mechanics to a `TokenConverter`.
/// It coordinates with payment conversion flows to prevent race conditions.
///
/// The active token can be changed at runtime via [`set_active_token`](Self::set_active_token).
/// When no token is active, all conversion operations are skipped.
#[derive(Clone)]
pub(crate) struct StableBalance {
    /// Configuration for stable balance behavior (shared across all tokens)
    pub(super) config: StableBalanceConfig,

    /// The currently active token, or `None` if deactivated
    pub(super) active_token: Arc<RwLock<Option<StableBalanceToken>>>,

    /// Reference to the token converter for executing conversions
    pub(super) token_converter: Arc<dyn TokenConverter>,

    /// Reference to the spark wallet for balance queries
    pub(super) spark_wallet: Arc<SparkWallet>,

    /// Reference to storage for checking existing conversions
    pub(super) storage: Arc<dyn Storage>,

    /// Cached effective values for auto-conversion (expires after TTL, shared across clones)
    pub(super) effective_values: Arc<ExpiringCell<EffectiveValues>>,

    /// Unified conversion queue for per-receive and auto-convert tasks.
    pub(super) queue: Arc<ConversionQueue>,

    /// Notify to signal first sync completion (startup gate for the conversion worker).
    pub(super) synced_notify: Arc<Notify>,

    /// Sync coordinator for triggering wallet syncs after conversions complete.
    pub(super) sync_coordinator: SyncCoordinator,

    /// Number of in-flight send-with-conversion payments.
    /// Auto-convert is suppressed while this is > 0.
    pub(super) payment_counter: Arc<AtomicUsize>,

    /// Lock that serializes "check counter + read balance" (auto-convert) with
    /// "increment counter" (payment start), preventing the balance read from
    /// seeing funds that an in-flight payment is about to spend.
    pub(super) payment_lock: Arc<Mutex<()>>,
}

impl StableBalance {
    /// Creates a new `StableBalance` instance and spawns background tasks.
    ///
    /// Resolves the initial active token from the local cache and config,
    /// and registers itself as an event listener on the provided emitter.
    pub async fn new(
        config: StableBalanceConfig,
        token_converter: Arc<dyn TokenConverter>,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        shutdown_receiver: watch::Receiver<()>,
        event_emitter: Arc<EventEmitter>,
        sync_coordinator: SyncCoordinator,
    ) -> Self {
        let initial_active_token = Self::resolve_initial_token(&config, &storage).await;

        let queue = Arc::new(ConversionQueue::new(storage.clone()));
        let synced_notify = Arc::new(Notify::new());

        if let Some(token) = &initial_active_token {
            info!(
                "Stable balance initialized with active token: {} ({})",
                token.label, token.token_identifier
            );
        } else {
            info!("Stable balance initialized as inactive");
        }

        let stable_balance = Self {
            config,
            active_token: Arc::new(RwLock::new(initial_active_token)),
            token_converter,
            spark_wallet,
            storage,
            effective_values: Arc::new(ExpiringCell::new()),
            queue,
            synced_notify,
            sync_coordinator,
            payment_counter: Arc::new(AtomicUsize::new(0)),
            payment_lock: Arc::new(Mutex::new(())),
        };

        // Register as event middleware
        event_emitter
            .add_middleware(Box::new(stable_balance.clone()))
            .await;

        // Spawn the unified conversion worker
        stable_balance.spawn_conversion_worker(shutdown_receiver);

        stable_balance
    }

    /// Returns the `token_identifier` of the currently active token, or `None` if inactive.
    pub(crate) async fn get_active_token_identifier(&self) -> Option<String> {
        self.active_token
            .read()
            .await
            .as_ref()
            .map(|t| t.token_identifier.clone())
    }

    /// Returns the label of the currently active token, or `None` if inactive.
    pub(crate) async fn get_active_label(&self) -> Option<String> {
        self.active_token
            .read()
            .await
            .as_ref()
            .map(|t| t.label.clone())
    }

    /// Acquires a payment guard that suppresses auto-convert while held.
    ///
    /// Call this before starting a send-with-conversion payment. The guard
    /// increments the payment counter; when dropped, it decrements the counter
    /// and wakes the conversion worker.
    pub(crate) async fn acquire_payment_guard(&self) -> PaymentGuard {
        // Hold the lock while incrementing so auto-convert's
        // "check counter + read balance" window can't interleave.
        let _lock = self.payment_lock.lock().await;
        self.payment_counter.fetch_add(1, Ordering::Relaxed);
        PaymentGuard {
            counter: self.payment_counter.clone(),
            notify: self.queue.notify.clone(),
        }
    }

    /// Sets the active token by label, or deactivates stable balance if `None`.
    ///
    /// Validates that the label exists in the configured tokens list.
    /// Clears the conversion queue (pending conversions for the old token are no longer
    /// relevant), marks cleared per-receive tasks as Failed, and caches the choice locally.
    pub(crate) async fn set_active_token(&self, label: Option<String>) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());

        // Clear the queue — pending conversions for the old token are no longer relevant
        let cleared_payment_ids = self.queue.clear_queue().await;
        if !cleared_payment_ids.is_empty() {
            info!(
                "Cleared {} pending conversion(s) from queue due to token change",
                cleared_payment_ids.len()
            );
        }
        for payment_id in &cleared_payment_ids {
            if let Err(e) = self
                .storage
                .insert_payment_metadata(
                    payment_id.clone(),
                    PaymentMetadata {
                        conversion_status: Some(ConversionStatus::Failed),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("Failed to persist Failed status for cleared conversion {payment_id}: {e:?}");
            }
        }

        let new_active = if let Some(label) = label {
            let token = self
                .config
                .tokens
                .iter()
                .find(|t| t.label == label)
                .ok_or_else(|| {
                    SdkError::InvalidInput(format!(
                        "Stable balance label '{label}' not found in configured tokens"
                    ))
                })?;
            cache.save_stable_balance_active_label(&label).await?;
            Some(token.clone())
        } else {
            // Deactivating — queue token-to-BTC conversion
            if let Some(current_token) = self.active_token.read().await.as_ref() {
                let token_id = current_token.token_identifier.clone();
                info!("Deactivating stable balance, queuing token-to-BTC conversion");
                self.queue.push_deactivation(token_id).await;
            }
            cache.delete_stable_balance_active_label().await?;
            None
        };

        if let Some(token) = &new_active {
            info!(
                "Stable balance active token changed to: {} ({})",
                token.label, token.token_identifier
            );
        } else {
            info!("Stable balance deactivated");
        }

        (*self.active_token.write().await).clone_from(&new_active);

        // Clear cached effective values since limits may differ per token
        self.effective_values.clear().await;

        // If enabling stable balance, trigger auto-convert for any existing excess
        if new_active.is_some() {
            self.queue.push_auto_convert().await;
        }

        Ok(())
    }

    /// Resolves the initial active token from the local cache and config.
    ///
    /// Resolution order:
    /// 1. If a cached label exists and is in the tokens list → use it
    /// 2. If a cached label exists but is NOT in the tokens list → inactive
    /// 3. If no cache exists → use `default_active_label` from config
    async fn resolve_initial_token(
        config: &StableBalanceConfig,
        storage: &Arc<dyn Storage>,
    ) -> Option<StableBalanceToken> {
        let cache = ObjectCacheRepository::new(storage.clone());

        match cache.fetch_stable_balance_active_label().await {
            Ok(Some(cached_label)) => {
                // Cached label exists — validate against config
                let token = config.tokens.iter().find(|t| t.label == cached_label);
                if token.is_none() {
                    info!(
                        "Cached stable balance label '{cached_label}' not found in config, deactivating"
                    );
                }
                token.cloned()
            }
            Ok(None) => {
                // No cache — use default from config
                config
                    .default_active_label
                    .as_ref()
                    .and_then(|label| config.tokens.iter().find(|t| t.label == *label).cloned())
            }
            Err(e) => {
                warn!("Failed to read stable balance cache: {e:?}, deactivating");
                None
            }
        }
    }

    /// Gets or initializes the effective threshold and min conversion limit for auto-conversion.
    ///
    /// Returns cached values if they exist and haven't expired. Otherwise, fetches
    /// conversion limits and computes:
    /// - Effective threshold: `max(user_threshold, min_from_amount)`
    pub(super) async fn get_or_init_effective_values(
        &self,
        active_token_identifier: &str,
    ) -> Result<(u64, u64), ConversionError> {
        // Return cached values if not expired
        if let Some(values) = self.effective_values.get().await {
            return Ok((values.threshold, values.min_from_amount));
        }

        // Fetch limits and compute effective values
        let limits = self
            .token_converter
            .fetch_limits(&FetchConversionLimitsRequest {
                conversion_type: ConversionType::FromBitcoin,
                token_identifier: Some(active_token_identifier.to_string()),
            })
            .await?;

        let min_from_amount =
            u64::try_from(limits.min_from_amount.unwrap_or(0)).unwrap_or(u64::MAX);

        // Compute effective threshold: max(user_threshold, min_from_amount)
        let threshold = match self.config.threshold_sats {
            Some(t) if t >= min_from_amount => t,
            Some(_) | None => min_from_amount,
        };

        // Cache with TTL
        self.effective_values
            .set(
                EffectiveValues {
                    threshold,
                    min_from_amount,
                },
                EFFECTIVE_VALUES_TTL_MS,
            )
            .await;
        info!(
            "Auto-conversion effective values initialized: threshold={threshold} sats, min_from_amount={min_from_amount} sats"
        );

        Ok((threshold, min_from_amount))
    }

    /// Gets conversion options for a payment if auto-population is needed.
    ///
    /// Returns `Some(ConversionOptions)` if:
    /// - Stable balance is active
    /// - No explicit options were provided
    /// - The payment is not a token payment (`token_identifier` is None)
    /// - The current sats balance is insufficient for the payment amount
    ///
    /// In this case, returns options to convert from the active stable token to Bitcoin.
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

        // Don't auto-convert if inactive
        let Some(active_token_identifier) = self.get_active_token_identifier().await else {
            return Ok(None);
        };

        let balance_sats = self.spark_wallet.get_balance().await?;

        // Only auto-populate if the sats balance is insufficient for the payment.
        if u128::from(balance_sats) >= payment_amount {
            return Ok(None);
        }

        info!(
            "Auto-populating conversion options: balance {balance_sats} sats < payment amount {payment_amount} sats"
        );
        Ok(Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: active_token_identifier,
            },
            max_slippage_bps: self.config.max_slippage_bps,
            completion_timeout_secs: None,
        }))
    }

    /// Checks if a payment should trigger per-receive conversion.
    ///
    /// Returns true if:
    /// - Payment is a receive type
    /// - Payment is not a token payment (i.e., it's a sats payment)
    /// - Stable balance is active
    /// - Payment amount meets the minimum conversion threshold
    async fn should_trigger_per_receive(&self, payment: &Payment) -> bool {
        if payment.payment_type != PaymentType::Receive || payment.method == PaymentMethod::Token {
            return false;
        }

        // Skip conversion child payments (e.g. intermediate sats from send-with-conversion)
        if payment.is_conversion_child() {
            return false;
        }

        let Some(token_id) = self.get_active_token_identifier().await else {
            return false;
        };

        let Ok((_, min_from_amount)) = self.get_or_init_effective_values(&token_id).await else {
            warn!("Failed to check effective values, skipping per-receive");
            return false;
        };

        let amount = u64::try_from(payment.amount).unwrap_or(u64::MAX);
        if amount < min_from_amount {
            debug!(
                "Skipping per-receive: amount {} < min {}",
                amount, min_from_amount
            );
            return false;
        }

        true
    }

    /// Triggers a full wallet sync so conversion payments and balance are updated.
    pub(super) async fn trigger_sync(&self) {
        use crate::sdk::SyncType;
        self.sync_coordinator
            .trigger_sync_no_wait(SyncType::Full, true)
            .await;
    }
}

#[macros::async_trait]
impl EventMiddleware for StableBalance {
    async fn process(&self, event: SdkEvent) -> Option<SdkEvent> {
        match event {
            // Sync completed → wake the startup gate, sweep timed-out deferred tasks
            SdkEvent::Synced => {
                // Clean up deferred tasks that have exceeded the timeout
                let expired_payment_ids = self.queue.clear_expired_tasks().await;
                for expired_payment_id in expired_payment_ids {
                    warn!("Per-receive conversion timed out for {expired_payment_id}");
                    if let Err(e) = self
                        .storage
                        .insert_payment_metadata(
                            expired_payment_id.clone(),
                            PaymentMetadata {
                                conversion_status: Some(ConversionStatus::Failed),
                                ..Default::default()
                            },
                        )
                        .await
                    {
                        warn!("Failed to persist Failed status for {expired_payment_id}: {e:?}");
                    }
                }

                self.synced_notify.notify_one();

                // Re-assess balance after sync — may have changed due to external activity
                self.queue.push_auto_convert().await;

                Some(SdkEvent::Synced)
            }

            // Payment succeeded → check if it resolves a deferred conversion,
            // then queue per-receive or auto-convert as needed
            SdkEvent::PaymentSucceeded { mut payment } => {
                // Check if this payment is a conversion result from another instance
                // that resolves a deferred per-receive task
                if let Some(parent_id) = self.queue.resolve_by_conversion_payment(&payment.id).await
                {
                    info!(
                        "Conversion payment {} resolved deferred task for {parent_id}",
                        payment.id
                    );
                    return Some(SdkEvent::PaymentSucceeded { payment });
                }

                if self.should_trigger_per_receive(&payment).await {
                    debug!("Queueing per-receive conversion for payment {}", payment.id);

                    // Set conversion_details with Pending status so clients know conversion is coming
                    payment.conversion_details = Some(ConversionDetails {
                        status: ConversionStatus::Pending,
                        from: None,
                        to: None,
                    });

                    // Persist the pending status so it survives restarts
                    if let Err(e) = self
                        .storage
                        .insert_payment_metadata(
                            payment.id.clone(),
                            PaymentMetadata {
                                conversion_status: Some(ConversionStatus::Pending),
                                ..Default::default()
                            },
                        )
                        .await
                    {
                        warn!(
                            "Failed to persist conversion_status for payment {}: {e:?}",
                            payment.id
                        );
                    }

                    self.queue.push_per_receive(payment.id.clone()).await;
                } else {
                    // Non-per-receive payment — queue auto-convert to handle accumulated balance
                    debug!("Queueing auto-convert after payment {}", payment.id);
                    self.queue.push_auto_convert().await;
                }
                Some(SdkEvent::PaymentSucceeded { payment })
            }

            _ => Some(event),
        }
    }
}
