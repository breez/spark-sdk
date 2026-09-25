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
//! │  2. Restore pending conversions, retry delay, and any owed          │
//! │     deactivation from the previous session                          │
//! │  3. Spawn conversion worker (waits for initial sync)                │
//! │  4. Pre-warm effective values cache (threshold, min limits)         │
//! │  5. Queue cold-start auto-convert                                   │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                          EVENT MIDDLEWARE                           │
//! │                                                                     │
//! │  PaymentSucceeded ──┬─► Sent leg of a deferred task? ──► Wake       │
//! │                     │                                               │
//! │                     ├─► Sats receive ≥ min, no delay? ─► PerReceive │
//! │                     │                                               │
//! │                     └─► Otherwise ──────────────────► AutoConvert   │
//! │                                                                     │
//! │  Synced ──────────────► Hand tasks deferred over 120s back to       │
//! │                         the worker to settle                        │
//! │                                                                     │
//! │  PaymentMetadataUpdated ► Sent leg of a deferred task? ► Wake       │
//! └─────────────────────────────────────────────────────────────────────┘
//!                                   │
//!                                   ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                          CONVERSION QUEUE                           │
//! │                                                                     │
//! │  Priority order:                                                    │
//! │    1. PerReceive(payment_id): convert individual received sats      │
//! │    2. Deactivation(token_id): convert active token back to BTC      │
//! │    3. AutoConvert: batch convert excess BTC balance                 │
//! │                                                                     │
//! │  Rules:                                                             │
//! │  • PerReceive deduplicates by payment_id                            │
//! │  • AutoConvert collapses multiple triggers into one                 │
//! │  • Deactivation overrides pending AutoConvert                       │
//! │  • AutoConvert/Deactivation only runs when no PerReceive pending    │
//! │    (including deferred: they may still need those sats)             │
//! │  • Deferred tasks are skipped until woken by their sent leg or      │
//! │    timed out                                                        │
//! │  • After a failed conversion no task is handed out until the        │
//! │    retry delay elapses: 30s, doubling to a cap of 1h. Any           │
//! │    conversion that succeeds, including one a user's own payment     │
//! │    drove, resets it, as does a change of active token. A woken      │
//! │    PerReceive still runs: it settles without converting             │
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
//! │    │  • A stored sent leg settles it from its record,        │      │
//! │    │    or the pool's listing when that record is silent     │      │
//! │    │  • On failure: settle from the sent leg's record, else  │      │
//! │    │    defer until the sent leg is seen or 120s pass        │      │
//! │    │  • Deferred over 120s: settles from the record or the   │      │
//! │    │    pool, else Failed. A pool that can't be asked is     │      │
//! │    │    retried for up to 1h                                 │      │
//! │    │  • Settles Completed or Failed, and on its own swap     │      │
//! │    │    also emits the completion event                      │      │
//! │    └─────────────────────────────────────────────────────────┘      │
//! │                                                                     │
//! │    ┌─────────────────────────────────────────────────────────┐      │
//! │    │ AutoConvert                                             │      │
//! │    │  • Check active token, ongoing payments, balance        │      │
//! │    │  • Acquire exclusive auto_conversion lock               │      │
//! │    │  • Check for token dust (would balance be below         │      │
//! │    │    ToBitcoin min limit?)                                │      │
//! │    │  • BTC → Token conversion (amount = full BTC balance)   │      │
//! │    │  • On success: emit completion event                    │      │
//! │    └─────────────────────────────────────────────────────────┘      │
//! │                                                                     │
//! │    ┌─────────────────────────────────────────────────────────┐      │
//! │    │ Deactivation                                            │      │
//! │    │  • Get token balance, check min conversion limit        │      │
//! │    │  • Acquire exclusive auto_conversion lock               │      │
//! │    │  • Token → BTC conversion (amount = full token balance) │      │
//! │    │  • On success: emit completion event, clear the owed    │      │
//! │    │    deactivation so a restart does not repeat it         │      │
//! │    └─────────────────────────────────────────────────────────┘      │
//! │                                                                     │
//! └─────────────────────────────────────────────────────────────────────┘
//!
//! Every failed conversion emits [`crate::SdkEvent::StableBalanceConversionFailed`]
//! with the delay before the next attempt, so a pair that cannot convert is
//! visible to the app rather than only slow.
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
//! │  AmountIn(sats): "convert exactly this much"                        │
//! │    Used by: PerReceive, AutoConvert, Deactivation                   │
//! │    Slippage applied to estimated output (conservative estimate)     │
//! │                                                                     │
//! │  MinAmountOut(sats): "I need at least this much out"                │
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
use tokio::sync::{Mutex, Notify, RwLock};
use tracing::{debug, info, warn};

use self::queue::ConversionQueue;
pub(crate) use self::queue::{ConversionBackoff, PendingQueue};

use crate::events::{EventEmitter, EventMiddleware, SdkEvent};
use crate::models::{
    ConversionDetails, ConversionStatus, Payment, PaymentMethod, PaymentType, StableBalanceToken,
};
use crate::persist::{ObjectCacheRepository, PaymentMetadata, Storage, StorageError};
use crate::sdk::RuntimeEvent;
use crate::utils::payments::emit_payment_metadata_updated;
use crate::{
    SdkError,
    models::StableBalanceConfig,
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

/// The conversion status stored on a payment, if any.
async fn stored_conversion_status(
    storage: &Arc<dyn Storage>,
    payment_id: &str,
) -> Option<ConversionStatus> {
    storage
        .get_payment_by_id(payment_id.to_string())
        .await
        .ok()?
        .conversion_details
        .map(|details| details.status)
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

/// State and logic shared between the emitter-owned [`StableBalanceMiddleware`]
/// and the [`StableBalance`] worker/API surface.
///
/// The middleware is owned by the `EventEmitter` for its whole lifetime, so
/// nothing reachable from this struct may hold a strong reference back to the
/// emitter, or the SDK object graph could never be dropped.
pub(crate) struct StableBalanceCore {
    /// Configuration for stable balance behavior (shared across all tokens)
    pub(super) config: StableBalanceConfig,

    /// The currently active token, or `None` if deactivated
    pub(super) active_token: RwLock<Option<StableBalanceToken>>,

    /// Reference to the token converter for executing conversions
    pub(super) token_converter: Arc<dyn TokenConverter>,

    /// Reference to storage for checking existing conversions
    pub(super) storage: Arc<dyn Storage>,

    /// Cached effective values for auto-conversion (expires after TTL)
    pub(super) effective_values: ExpiringCell<EffectiveValues>,

    /// Unified conversion queue for per-receive and auto-convert tasks.
    pub(super) queue: ConversionQueue,

    /// Notify to signal first sync completion (startup gate for the conversion worker).
    pub(super) synced_notify: Notify,

    /// Held across each guarded status write, so one writer's check and write
    /// cannot interleave with another's.
    pub(super) status_lock: Mutex<()>,
}

/// Event middleware that feeds the conversion queue from payment and sync
/// events. Owned by the `EventEmitter`; never emits and holds no emitter
/// reference.
pub(crate) struct StableBalanceMiddleware {
    core: Arc<StableBalanceCore>,
}

/// Manages stable balance auto-conversion behavior.
///
/// This struct handles the business logic of when and how much to convert,
/// while delegating the actual conversion mechanics to a `TokenConverter`.
/// It coordinates with payment conversion flows to prevent race conditions.
///
/// Held by `BreezSdk` and the conversion worker task (which stops on
/// shutdown), never by the emitter itself: the emitter owns only the
/// [`StableBalanceMiddleware`], so the strong emitter reference here cannot
/// form a cycle.
///
/// The active token can be changed at runtime via [`set_active_token`](Self::set_active_token).
/// When no token is active, all conversion operations are skipped.
#[derive(Clone)]
pub(crate) struct StableBalance {
    /// State shared with the registered [`StableBalanceMiddleware`].
    pub(super) core: Arc<StableBalanceCore>,

    /// Reference to the spark wallet for balance queries
    pub(super) spark_wallet: Arc<SparkWallet>,

    /// Event emitter used to publish conversion outcomes to the active runtime.
    pub(super) event_emitter: Arc<EventEmitter>,

    /// Number of in-flight send-with-conversion payments.
    /// Auto-convert is suppressed while this is > 0.
    pub(super) payment_counter: Arc<AtomicUsize>,

    /// Lock that serializes "check counter + read balance" (auto-convert) with
    /// "increment counter" (payment start), preventing the balance read from
    /// seeing funds that an in-flight payment is about to spend.
    pub(super) payment_lock: Arc<Mutex<()>>,
}

impl StableBalance {
    /// Creates a new `StableBalance` instance.
    ///
    /// Resolves the initial active token from the local cache and config,
    /// and registers a [`StableBalanceMiddleware`] on the provided emitter.
    pub async fn new(
        config: StableBalanceConfig,
        token_converter: Arc<dyn TokenConverter>,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        event_emitter: Arc<EventEmitter>,
    ) -> Self {
        let initial_active_token =
            StableBalanceCore::resolve_initial_token(&config, &storage).await;

        if let Some(token) = &initial_active_token {
            info!(
                "Stable balance initialized with active token: {} ({})",
                token.label, token.token_identifier
            );
        } else {
            info!("Stable balance initialized as inactive");
        }

        let core = Arc::new(StableBalanceCore {
            config,
            active_token: RwLock::new(initial_active_token),
            token_converter,
            storage: Arc::clone(&storage),
            effective_values: ExpiringCell::new(),
            queue: ConversionQueue::load(storage).await,
            synced_notify: Notify::new(),
            status_lock: Mutex::new(()),
        });

        event_emitter
            .add_middleware(Box::new(StableBalanceMiddleware {
                core: Arc::clone(&core),
            }))
            .await;

        Self {
            core,
            spark_wallet,
            event_emitter,
            payment_counter: Arc::new(AtomicUsize::new(0)),
            payment_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Returns the `token_identifier` of the currently active token, or `None` if inactive.
    pub(crate) async fn get_active_token_identifier(&self) -> Option<String> {
        self.core.get_active_token_identifier().await
    }

    /// Returns the label of the currently active token, or `None` if inactive.
    pub(crate) async fn get_active_label(&self) -> Option<String> {
        self.core.get_active_label().await
    }

    /// Sets the active token by label, or deactivates stable balance if `None`.
    ///
    /// Pending conversions for the old token are no longer relevant: the
    /// queue is cleared and cleared per-receive tasks are marked Failed.
    pub(crate) async fn set_active_token(&self, label: Option<String>) -> Result<(), SdkError> {
        self.core.set_active_token(label, &self.event_emitter).await
    }

    /// Clears the retry delay when a conversion outside the worker succeeded
    /// against the token the worker converts. A different token rides a
    /// different pool and says nothing about this one.
    pub(crate) async fn clear_conversion_backoff_for(&self, token_identifier: Option<String>) {
        if token_identifier.is_some()
            && token_identifier == self.core.get_active_token_identifier().await
        {
            self.core.queue.clear_backoff().await;
        }
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
            notify: self.core.queue.notify.clone(),
        }
    }
}

impl StableBalanceCore {
    /// Writes `status` onto a payment unless the stored status outranks it.
    /// `Completed` records that the swap ran, so it replaces any other status
    /// and is never replaced by this device. Any other status replaces only
    /// `Pending`, and `Pending` is written only where no status is stored.
    /// Returns whether it wrote.
    pub(super) async fn finalize_conversion_status(
        &self,
        payment_id: &str,
        status: ConversionStatus,
    ) -> Result<bool, StorageError> {
        let _guard = self.status_lock.lock().await;
        let stored = stored_conversion_status(&self.storage, payment_id).await;
        let writable = match (stored, &status) {
            (Some(ConversionStatus::Completed), _) => false,
            (None, _) | (Some(_), ConversionStatus::Completed) => true,
            (Some(ConversionStatus::Pending), _) => status != ConversionStatus::Pending,
            (Some(_), _) => false,
        };
        if !writable {
            debug!("Keeping the stored status on {payment_id} instead of {status:?}");
            return Ok(false);
        }
        self.storage
            .insert_payment_metadata(
                payment_id.to_string(),
                PaymentMetadata {
                    conversion_status: Some(status.clone()),
                    ..Default::default()
                },
            )
            .await
            .inspect_err(|e| {
                warn!("Failed to persist {status:?} status for {payment_id}: {e:?}");
            })?;
        Ok(true)
    }

    /// Finalises a status and emits `PaymentMetadataUpdated` when it was
    /// written.
    pub(super) async fn finalize_and_emit(
        &self,
        event_emitter: &EventEmitter,
        payment_id: &str,
        status: ConversionStatus,
    ) -> Result<(), StorageError> {
        if self.finalize_conversion_status(payment_id, status).await? {
            emit_payment_metadata_updated(&self.storage, event_emitter, payment_id).await;
        }
        Ok(())
    }

    /// Returns the `token_identifier` of the currently active token, or `None` if inactive.
    pub(super) async fn get_active_token_identifier(&self) -> Option<String> {
        self.active_token
            .read()
            .await
            .as_ref()
            .map(|t| t.token_identifier.clone())
    }

    /// Returns the label of the currently active token, or `None` if inactive.
    async fn get_active_label(&self) -> Option<String> {
        self.active_token
            .read()
            .await
            .as_ref()
            .map(|t| t.label.clone())
    }

    /// Sets the active token by label, or deactivates stable balance if `None`.
    ///
    /// The label is validated before anything is cleared, so an unknown label
    /// leaves the queue and its payments untouched.
    async fn set_active_token(
        &self,
        label: Option<String>,
        event_emitter: &EventEmitter,
    ) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());

        // Resolved before anything is cleared: a label that is not configured
        // must leave the queue and its payments alone.
        let new_token = match &label {
            Some(label) => Some(
                self.config
                    .tokens
                    .iter()
                    .find(|t| t.label == *label)
                    .ok_or_else(|| {
                        SdkError::InvalidInput(format!(
                            "Stable balance label '{label}' not found in configured tokens"
                        ))
                    })?,
            ),
            None => None,
        };

        let cleared_payment_ids = self
            .queue
            .clear_for_token_change(new_token.map(|t| t.token_identifier.as_str()))
            .await;
        if !cleared_payment_ids.is_empty() {
            info!(
                "Cleared {} pending conversion(s) from queue due to token change",
                cleared_payment_ids.len()
            );
        }
        for payment_id in &cleared_payment_ids {
            let _ = self
                .finalize_and_emit(event_emitter, payment_id, ConversionStatus::Failed)
                .await;
        }

        // Held across the queue push and the label write. The worker's
        // deactivation guard reads the active token, and blocking it here is
        // what stops it observing the token as still active once the task is
        // queued, which would cancel the conversion.
        let mut active = self.active_token.write().await;

        let new_active = if let Some(token) = new_token {
            cache.save_stable_balance_active_label(&token.label).await?;
            Some(token.clone())
        } else {
            if let Some(token_id) = active.as_ref().map(|t| t.token_identifier.clone()) {
                info!("Deactivating stable balance, queuing token-to-BTC conversion");
                // Queued and persisted before the label is cleared, so no
                // crash can leave the label off with the conversion unqueued.
                self.queue.push_deactivation(token_id).await;
            }
            cache.save_stable_balance_deactivated().await?;
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

        active.clone_from(&new_active);

        // Clear cached effective values since limits may differ per token
        self.effective_values.clear().await;

        // Reset the failure count so the next conversion runs at once rather
        // than waiting out a delay the previous token built up.
        self.queue.clear_backoff().await;

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
            // Turned off by the user, which outranks the configured default.
            Ok(Some(cached_label)) if cached_label.is_empty() => None,
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

    /// Checks if a payment should trigger per-receive conversion.
    ///
    /// Returns true if:
    /// - Payment is a receive type
    /// - Payment is not a token payment (i.e., it's a sats payment)
    /// - Stable balance is active
    /// - Payment amount meets the minimum conversion threshold
    /// - No retry delay from a previous failure is running
    async fn should_trigger_per_receive(&self, payment: &Payment) -> bool {
        if payment.payment_type != PaymentType::Receive || payment.method == PaymentMethod::Token {
            return false;
        }

        // Skip conversion child payments (e.g. intermediate sats from send-with-conversion)
        if payment.is_conversion_child() {
            return false;
        }

        // While conversions are delayed after a failure, the sats go to the
        // batch conversion instead, which waits out the delay without marking
        // the payment pending.
        if self.queue.retry_delay().await.is_some() {
            debug!(
                "Skipping per-receive for {}: conversions are delayed",
                payment.id
            );
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
}

impl StableBalance {
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
        let Some(active_token_identifier) = self.core.get_active_token_identifier().await else {
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
            max_slippage_bps: self.core.config.max_slippage_bps,
            completion_timeout_secs: None,
        }))
    }

    /// Emits the fact that a conversion changed balances and payments.
    pub(super) async fn emit_conversion_completed(&self) {
        self.event_emitter
            .emit_runtime_event(RuntimeEvent::StableBalanceConversionCompleted)
            .await;
    }
}

impl StableBalanceMiddleware {
    /// Wakes the deferred task whose sent leg `payment_id` is. Returns whether one
    /// was woken. The worker settles it: this middleware holds no emitter.
    async fn wake_deferred(&self, payment_id: &str) -> bool {
        let Some(parent_id) = self.core.queue.wake_by_sent_leg(payment_id).await else {
            return false;
        };
        info!("Sent leg {payment_id} woke the deferred conversion for {parent_id}");
        true
    }
}

#[macros::async_trait]
impl EventMiddleware for StableBalanceMiddleware {
    async fn process(&self, event: SdkEvent) -> Option<SdkEvent> {
        match event {
            // Sync completed → wake the startup gate, hand timed-out tasks back
            SdkEvent::Synced => {
                // The worker settles deferred tasks past the timeout
                let timed_out = self.core.queue.wake_expired_tasks().await;
                if !timed_out.is_empty() {
                    debug!(
                        "Handing timed out per-receive conversions to the worker: {timed_out:?}"
                    );
                }

                self.core.synced_notify.notify_one();

                // Re-assess balance after sync — may have changed due to external activity
                self.core.queue.push_auto_convert().await;

                Some(SdkEvent::Synced)
            }

            // Metadata synced in from another device may say how a deferred
            // task's swap ended.
            SdkEvent::PaymentMetadataUpdated { payment } => {
                self.wake_deferred(&payment.id).await;
                Some(SdkEvent::PaymentMetadataUpdated { payment })
            }

            // Payment succeeded → wake a deferred conversion whose sent leg this
            // is, or queue per-receive or auto-convert as needed
            SdkEvent::PaymentSucceeded { mut payment } => {
                if self.wake_deferred(&payment.id).await {
                    return Some(SdkEvent::PaymentSucceeded { payment });
                }

                // A payment another device already settled is left alone.
                let per_receive = self.core.should_trigger_per_receive(&payment).await
                    && !stored_conversion_status(&self.core.storage, &payment.id)
                        .await
                        .is_some_and(|status| status != ConversionStatus::Pending);
                if per_receive {
                    debug!("Queueing per-receive conversion for payment {}", payment.id);

                    // Set conversion_details with Pending status so clients know conversion is coming
                    payment.conversion_details = Some(ConversionDetails {
                        status: ConversionStatus::Pending,
                        conversions: vec![],
                    });

                    // Persist the pending status so it survives restarts
                    let _ = self
                        .core
                        .finalize_conversion_status(&payment.id, ConversionStatus::Pending)
                        .await;

                    self.core.queue.push_per_receive(payment.id.clone()).await;
                } else {
                    // Non-per-receive payment — queue auto-convert to handle accumulated balance
                    debug!("Queueing auto-convert after payment {}", payment.id);
                    self.core.queue.push_auto_convert().await;
                }
                Some(SdkEvent::PaymentSucceeded { payment })
            }

            _ => Some(event),
        }
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::persist::sqlite::SqliteStorage;
    use crate::token_conversion::{
        ConversionAmount, ConversionOptions, ConversionPurpose, FetchConversionLimitsRequest,
        FetchConversionLimitsResponse, TokenConversionResponse, TokenConverter,
    };
    use spark_wallet::TransferId;

    /// Never converts. Reports the legs in `completed_legs` as a swap that
    /// already ran, when set, and fails every lookup when `lookup_fails`.
    #[derive(Default)]
    struct StubConverter {
        completed_legs: Option<(&'static str, &'static str)>,
        lookup_fails: bool,
    }

    #[macros::async_trait]
    impl TokenConverter for StubConverter {
        async fn convert(
            &self,
            _: Arc<EventEmitter>,
            _: &ConversionOptions,
            _: &ConversionPurpose,
            _: Option<&String>,
            _: ConversionAmount,
            _: Option<TransferId>,
        ) -> Result<TokenConversionResponse, ConversionError> {
            unreachable!("the rejected path must not convert")
        }

        async fn find_completed_conversion(
            &self,
            _: &TransferId,
            _: &ConversionPurpose,
        ) -> Result<Option<TokenConversionResponse>, ConversionError> {
            if self.lookup_fails {
                return Err(ConversionError::ConversionFailed(
                    "swaps could not be listed".to_string(),
                ));
            }
            Ok(self
                .completed_legs
                .map(|(sent, received)| TokenConversionResponse {
                    sent_payment_id: sent.to_string(),
                    received_payment_id: received.to_string(),
                }))
        }

        async fn validate(
            &self,
            _: Option<&ConversionOptions>,
            _: Option<&String>,
            _: ConversionAmount,
        ) -> Result<Option<crate::token_conversion::ConversionEstimate>, ConversionError> {
            Ok(None)
        }

        async fn fetch_limits(
            &self,
            _: &FetchConversionLimitsRequest,
        ) -> Result<FetchConversionLimitsResponse, ConversionError> {
            Ok(FetchConversionLimitsResponse {
                min_from_amount: None,
                min_to_amount: None,
            })
        }

        async fn refund_pending(
            &self,
        ) -> Result<crate::RefundPendingConversionsResponse, ConversionError> {
            Ok(crate::RefundPendingConversionsResponse::default())
        }

        async fn refund_local_pending(
            &self,
        ) -> Result<crate::RefundPendingConversionsResponse, ConversionError> {
            Ok(crate::RefundPendingConversionsResponse::default())
        }
    }

    fn usd_config(default_active_label: Option<&str>) -> StableBalanceConfig {
        StableBalanceConfig {
            tokens: vec![StableBalanceToken {
                label: "USD".to_string(),
                token_identifier: "token-usd".to_string(),
            }],
            default_active_label: default_active_label.map(str::to_string),
            threshold_sats: None,
            max_slippage_bps: None,
        }
    }

    fn label_storage(name: &str) -> Arc<dyn Storage> {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Arc::new(SqliteStorage::new(&path).unwrap())
    }

    /// Turning stable balance off must survive a restart even when the config
    /// names a default: the user's choice takes precedence.
    #[tokio::test]
    async fn turning_off_outlasts_the_configured_default() {
        let storage = label_storage("off-outlasts-default");
        ObjectCacheRepository::new(Arc::clone(&storage))
            .save_stable_balance_deactivated()
            .await
            .unwrap();

        let token =
            StableBalanceCore::resolve_initial_token(&usd_config(Some("USD")), &storage).await;
        assert!(token.is_none());
    }

    #[tokio::test]
    async fn with_no_choice_made_the_configured_default_applies() {
        let storage = label_storage("default-applies");
        let token =
            StableBalanceCore::resolve_initial_token(&usd_config(Some("USD")), &storage).await;
        assert_eq!(token.map(|t| t.label), Some("USD".to_string()));
    }

    /// While conversions are delayed, a received payment is left to the batch
    /// conversion instead of being queued and marked pending for the whole
    /// delay.
    #[tokio::test]
    async fn a_payment_received_during_a_delay_is_not_taken_per_receive() {
        let usd = StableBalanceToken {
            label: "USD".to_string(),
            token_identifier: "token-usd".to_string(),
        };
        let storage = label_storage("per-receive-during-delay");
        let core = StableBalanceCore {
            config: usd_config(Some("USD")),
            active_token: RwLock::new(Some(usd)),
            token_converter: Arc::new(StubConverter::default()),
            storage: Arc::clone(&storage),
            effective_values: ExpiringCell::new(),
            queue: ConversionQueue::new(storage),
            synced_notify: Notify::new(),
            status_lock: Mutex::new(()),
        };
        let payment = Payment {
            id: "received".to_string(),
            payment_type: PaymentType::Receive,
            status: crate::PaymentStatus::Completed,
            amount: 5_000,
            fees: 0,
            timestamp: 1,
            method: PaymentMethod::Spark,
            details: None,
            conversion_details: None,
        };

        assert!(core.should_trigger_per_receive(&payment).await);

        core.queue.record_failure().await;
        assert!(!core.should_trigger_per_receive(&payment).await);
    }

    /// A label that is not configured is rejected without draining the queue
    /// or marking its payments failed.
    #[tokio::test]
    async fn an_unknown_label_leaves_the_queue_alone() {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-label-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&path).unwrap());

        let core = StableBalanceCore {
            config: StableBalanceConfig {
                tokens: vec![StableBalanceToken {
                    label: "USD".to_string(),
                    token_identifier: "token-usd".to_string(),
                }],
                default_active_label: None,
                threshold_sats: None,
                max_slippage_bps: None,
            },
            active_token: RwLock::new(None),
            token_converter: Arc::new(StubConverter::default()),
            storage: Arc::clone(&storage),
            effective_values: ExpiringCell::new(),
            queue: ConversionQueue::new(storage),
            synced_notify: Notify::new(),
            status_lock: Mutex::new(()),
        };

        core.queue.push_per_receive("payment-1".to_string()).await;
        assert!(
            core.set_active_token(Some("nope".to_string()), &EventEmitter::new(false))
                .await
                .is_err()
        );

        assert!(
            core.queue.next_task().await.is_some(),
            "a rejected change must not drain the queue"
        );
    }

    /// A core with USD active, and the queue the storage holds.
    async fn active_core(storage: Arc<dyn Storage>) -> Arc<StableBalanceCore> {
        core_with(storage, StubConverter::default()).await
    }

    async fn core_with(
        storage: Arc<dyn Storage>,
        converter: StubConverter,
    ) -> Arc<StableBalanceCore> {
        Arc::new(StableBalanceCore {
            config: usd_config(Some("USD")),
            active_token: RwLock::new(Some(StableBalanceToken {
                label: "USD".to_string(),
                token_identifier: "token-usd".to_string(),
            })),
            token_converter: Arc::new(converter),
            storage: Arc::clone(&storage),
            effective_values: ExpiringCell::new(),
            queue: ConversionQueue::load(storage).await,
            synced_notify: Notify::new(),
            status_lock: Mutex::new(()),
        })
    }

    fn payment(id: &str, payment_type: PaymentType) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type,
            status: crate::PaymentStatus::Completed,
            amount: 5_000,
            fees: 0,
            timestamp: 1,
            method: PaymentMethod::Spark,
            details: None,
            conversion_details: None,
        }
    }

    async fn store(storages: &[&Arc<dyn Storage>], ids: &[&str]) {
        for storage in storages {
            for id in ids {
                storage
                    .apply_payment_update(payment(id, PaymentType::Receive))
                    .await
                    .unwrap();
            }
        }
    }

    /// The sent leg of a deferred task hands it back to the worker, which is the
    /// only side able to announce the status it settles on.
    #[tokio::test]
    async fn the_middleware_wakes_a_deferred_task_without_settling_it() {
        let storage = label_storage("middleware-wake");
        let core = active_core(Arc::clone(&storage)).await;
        let middleware = StableBalanceMiddleware {
            core: Arc::clone(&core),
        };
        store(&[&storage], &["received"]).await;
        core.finalize_conversion_status("received", ConversionStatus::Pending)
            .await
            .unwrap();
        core.queue.push_per_receive("received".to_string()).await;
        let sent_leg = per_receive_transfer_id("received").to_string();

        for event in [
            SdkEvent::PaymentSucceeded {
                payment: payment(&sent_leg, PaymentType::Send),
            },
            SdkEvent::PaymentMetadataUpdated {
                payment: payment(&sent_leg, PaymentType::Send),
            },
        ] {
            core.queue.defer_task("received").await;
            assert!(middleware.process(event).await.is_some());
            assert_eq!(
                core.queue.next_task().await,
                Some(queue::ConversionTask::PerReceive("received".to_string()))
            );
            assert_eq!(
                stored_conversion_status(&storage, "received").await,
                Some(ConversionStatus::Pending)
            );
        }
    }

    /// An arrival is marked `Pending` and queued. One whose status is already
    /// settled, for example by another device over sync, keeps that status and
    /// is not queued again.
    #[tokio::test]
    async fn an_arrival_is_marked_pending_and_a_settled_one_is_left_alone() {
        let storage = label_storage("pending-arrival");
        let core = active_core(Arc::clone(&storage)).await;
        let middleware = StableBalanceMiddleware {
            core: Arc::clone(&core),
        };
        store(&[&storage], &["fresh", "settled"]).await;
        core.finalize_conversion_status("settled", ConversionStatus::Completed)
            .await
            .unwrap();

        let Some(SdkEvent::PaymentSucceeded { payment: fresh }) = middleware
            .process(SdkEvent::PaymentSucceeded {
                payment: payment("fresh", PaymentType::Receive),
            })
            .await
        else {
            panic!("the event must pass through");
        };
        assert_eq!(
            fresh.conversion_details.map(|d| d.status),
            Some(ConversionStatus::Pending)
        );
        assert_eq!(
            stored_conversion_status(&storage, "fresh").await,
            Some(ConversionStatus::Pending)
        );

        let Some(SdkEvent::PaymentSucceeded { payment: settled }) = middleware
            .process(SdkEvent::PaymentSucceeded {
                payment: payment("settled", PaymentType::Receive),
            })
            .await
        else {
            panic!("the event must pass through");
        };
        assert!(settled.conversion_details.is_none());
        assert_eq!(
            stored_conversion_status(&storage, "settled").await,
            Some(ConversionStatus::Completed)
        );

        let fresh = queue::ConversionTask::PerReceive("fresh".to_string());
        assert_eq!(core.queue.next_task().await, Some(fresh.clone()));
        core.queue.complete_task(&fresh).await;
        assert!(
            !core.queue.has_per_receive().await,
            "the settled payment is not queued"
        );
    }

    /// `Completed` records that the swap ran, so it replaces a status written
    /// without that knowledge and is never replaced. Other statuses replace
    /// only `Pending`.
    #[tokio::test]
    async fn completed_outranks_every_other_status() {
        let storage = label_storage("finalize-guard");
        store(&[&storage], &["received"]).await;
        let core = active_core(Arc::clone(&storage)).await;
        let finalize = |status| core.finalize_conversion_status("received", status);

        assert!(finalize(ConversionStatus::Pending).await.unwrap());
        assert!(!finalize(ConversionStatus::Pending).await.unwrap());
        assert!(finalize(ConversionStatus::Failed).await.unwrap());
        assert!(!finalize(ConversionStatus::Pending).await.unwrap());
        assert!(finalize(ConversionStatus::Completed).await.unwrap());
        assert!(!finalize(ConversionStatus::Failed).await.unwrap());
        assert!(!finalize(ConversionStatus::Completed).await.unwrap());
        assert_eq!(
            stored_conversion_status(&storage, "received").await,
            Some(ConversionStatus::Completed)
        );
    }

    /// Stores each parent with a pending conversion, its sent leg when a status
    /// is given, and a deferred task for it queued `age_secs` ago.
    async fn seed_timed_out(
        storage: &Arc<dyn Storage>,
        tasks: &[(&str, Option<ConversionStatus>)],
        age_secs: u64,
    ) {
        let expired_at = crate::utils::time::now_secs().saturating_sub(age_secs);
        let mut per_receive = Vec::new();
        for (parent, sent_leg_status) in tasks {
            store(&[storage], &[parent]).await;
            storage
                .insert_payment_metadata(
                    parent.to_string(),
                    PaymentMetadata {
                        conversion_status: Some(ConversionStatus::Pending),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            if let Some(status) = sent_leg_status {
                let sent_leg_id = per_receive_transfer_id(parent).to_string();
                storage
                    .apply_payment_update(Payment {
                        details: Some(crate::PaymentDetails::Spark {
                            invoice_details: None,
                            htlc_details: None,
                            conversion_info: None,
                        }),
                        ..payment(&sent_leg_id, PaymentType::Send)
                    })
                    .await
                    .unwrap();
                storage
                    .insert_payment_metadata(
                        sent_leg_id,
                        PaymentMetadata {
                            conversion_info: Some(crate::ConversionInfo::Amm {
                                pool_id: "pool".to_string(),
                                conversion_id: "conversion".to_string(),
                                status: status.clone(),
                                fee: None,
                                purpose: None,
                                amount_adjustment: None,
                                degradation: None,
                            }),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
            }
            per_receive.push(serde_json::json!({
                "payment_id": parent,
                "state": "Deferred",
                "created_at": expired_at,
            }));
        }
        let queue: PendingQueue = serde_json::from_value(serde_json::json!({
            "per_receive": per_receive,
            "deactivations": [],
        }))
        .unwrap();
        ObjectCacheRepository::new(Arc::clone(storage))
            .save_pending_conversions(&queue)
            .await
            .unwrap();
    }

    /// The sweep only hands a task past the timeout back. The worker settles
    /// it from its sent leg's record, and asks the pool when that record is
    /// silent or no sent leg is stored.
    #[tokio::test]
    async fn a_timed_out_task_settles_from_what_is_known_of_its_swap() {
        let storage = label_storage("timed-out-settle");
        seed_timed_out(
            &storage,
            &[
                ("converted", Some(ConversionStatus::Completed)),
                ("refunding", Some(ConversionStatus::RefundNeeded)),
                ("unrecorded", None),
            ],
            1_000,
        )
        .await;
        let core = core_with(
            Arc::clone(&storage),
            StubConverter {
                completed_legs: Some(("sent-leg", "received-leg")),
                ..Default::default()
            },
        )
        .await;
        let middleware = StableBalanceMiddleware {
            core: Arc::clone(&core),
        };

        middleware.process(SdkEvent::Synced).await;
        for parent in ["converted", "refunding", "unrecorded"] {
            assert!(core.queue.is_timed_out(parent).await, "{parent}");
            assert_eq!(
                stored_conversion_status(&storage, parent).await,
                Some(ConversionStatus::Pending),
                "the sweep itself settles nothing"
            );
            assert!(
                core.settle_timed_out(&EventEmitter::new(false), parent)
                    .await
            );
            assert_eq!(
                stored_conversion_status(&storage, parent).await,
                Some(ConversionStatus::Completed),
                "{parent}"
            );
        }
    }

    /// When neither the sent leg's record nor the pool says the swap ran, a task
    /// past the timeout settles Failed.
    #[tokio::test]
    async fn a_timed_out_task_the_pool_does_not_report_settles_failed() {
        let storage = label_storage("timed-out-unreported");
        seed_timed_out(
            &storage,
            &[
                ("refunding", Some(ConversionStatus::RefundNeeded)),
                ("unsent", None),
            ],
            1_000,
        )
        .await;
        let core = active_core(Arc::clone(&storage)).await;

        for parent in ["refunding", "unsent"] {
            assert!(
                core.settle_timed_out(&EventEmitter::new(false), parent)
                    .await
            );
            assert_eq!(
                stored_conversion_status(&storage, parent).await,
                Some(ConversionStatus::Failed),
                "{parent}"
            );
        }
    }

    /// When the pool cannot be asked, a task past the timeout is left pending
    /// to be tried again, rather than settled on a guess.
    #[tokio::test]
    async fn a_timed_out_task_waits_when_the_pool_cannot_be_asked() {
        let storage = label_storage("timed-out-lookup-fails");
        seed_timed_out(
            &storage,
            &[
                ("refunding", Some(ConversionStatus::RefundNeeded)),
                ("unsent", None),
            ],
            1_000,
        )
        .await;
        let core = core_with(
            Arc::clone(&storage),
            StubConverter {
                lookup_fails: true,
                ..Default::default()
            },
        )
        .await;

        for parent in ["refunding", "unsent"] {
            assert!(
                !core
                    .settle_timed_out(&EventEmitter::new(false), parent)
                    .await
            );
            assert_eq!(
                stored_conversion_status(&storage, parent).await,
                Some(ConversionStatus::Pending),
                "{parent}"
            );
        }
    }

    /// Once the lookup deadline has passed, a task whose pool lookup keeps
    /// failing settles Failed rather than staying pending.
    #[tokio::test]
    async fn a_timed_out_task_settles_failed_once_the_lookup_deadline_passes() {
        let storage = label_storage("timed-out-lookup-deadline");
        seed_timed_out(
            &storage,
            &[("refunding", Some(ConversionStatus::RefundNeeded))],
            2 * 3_600,
        )
        .await;
        let core = core_with(
            Arc::clone(&storage),
            StubConverter {
                lookup_fails: true,
                ..Default::default()
            },
        )
        .await;

        assert!(
            core.settle_timed_out(&EventEmitter::new(false), "refunding")
                .await
        );
        assert_eq!(
            stored_conversion_status(&storage, "refunding").await,
            Some(ConversionStatus::Failed)
        );
    }
}
