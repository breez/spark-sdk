//! Stable balance management for automatic BTC-to-token conversion.
//!
//! This module provides functionality to maintain a "stable balance" by automatically
//! converting received BTC to a configured stable token when thresholds are exceeded.
//! The active token can be changed at runtime via [`crate::models::UpdateUserSettingsRequest`].

mod auto_conversion;
mod receive_conversion;

use std::sync::Arc;

use spark_wallet::SparkWallet;
use tokio::sync::{Notify, RwLock, mpsc, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info, warn};

use breez_sdk_common::sync::SigningClient;

use crate::events::{EventListener, SdkEvent};
use crate::models::{Payment, PaymentMethod, PaymentType, StableBalanceToken};
use crate::persist::{ObjectCacheRepository, Storage};
use crate::realtime_sync::sync_lock::{LockCounter, SyncLockGuard};
use crate::{
    SdkError,
    models::StableBalanceConfig,
    token_conversion::{
        ConversionError, ConversionOptions, ConversionType, FetchConversionLimitsRequest,
        TokenConverter,
    },
    utils::expiring_cell::ExpiringCell,
};

/// Lock name for payment conversion guards (non-exclusive, fire-and-forget).
pub(super) const PAYMENT_LOCK_NAME: &str = "payments_conversion";
/// Lock name for auto-conversion (exclusive).
pub(super) const AUTO_CONVERT_LOCK_NAME: &str = "auto_conversion";
/// TTL for cached effective values (1 hour)
pub(super) const EFFECTIVE_VALUES_TTL_MS: u128 = 3_600_000;

/// Cached effective threshold and reserved values for auto-conversion.
#[derive(Clone)]
pub(super) struct EffectiveValues {
    pub threshold: u64,
    pub reserved: u64,
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

    /// Tracks the number of in-flight payment conversions.
    /// Auto-convert is skipped while any payments are ongoing.
    pub(super) ongoing_payments: Arc<LockCounter>,

    /// Notify to trigger auto-conversion
    pub(super) auto_convert_trigger: Arc<Notify>,

    /// Notify to signal first sync completion (for receive conversion startup safety)
    pub(super) synced_notify: Arc<Notify>,

    /// Sender for per-receive conversion tasks (payment IDs)
    pub(super) per_receive_tx: mpsc::UnboundedSender<String>,

    /// Optional signing client for coordinating across SDK instances.
    /// `None` when real-time sync is not configured.
    pub(super) signing_client: Option<SigningClient>,
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
        signing_client: Option<SigningClient>,
        event_emitter: &crate::events::EventEmitter,
    ) -> Self {
        let initial_active_token = Self::resolve_initial_token(&config, &storage).await;

        let auto_convert_trigger = Arc::new(Notify::new());
        let synced_notify = Arc::new(Notify::new());
        let (per_receive_tx, per_receive_rx) = mpsc::unbounded_channel();

        if let Some(token) = &initial_active_token {
            info!(
                "Stable balance initialized with active token: {} ({})",
                token.ticker, token.token_identifier
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
            ongoing_payments: Arc::new(LockCounter::new()),
            auto_convert_trigger,
            synced_notify,
            per_receive_tx,
            signing_client,
        };

        // Spawn the background tasks
        stable_balance.spawn_auto_convert_task(shutdown_receiver.clone());
        stable_balance.spawn_receive_convert_task(per_receive_rx, shutdown_receiver);

        // Register as event listener
        event_emitter
            .add_listener(Box::new(stable_balance.clone()))
            .await;

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

    /// Returns the ticker of the currently active token, or `None` if inactive.
    pub(crate) async fn get_active_ticker(&self) -> Option<String> {
        self.active_token
            .read()
            .await
            .as_ref()
            .map(|t| t.ticker.clone())
    }

    /// Sets the active token by ticker, or deactivates stable balance if `None`.
    ///
    /// Validates that the ticker exists in the configured tokens list.
    /// Caches the choice locally and clears the effective values cache.
    pub(crate) async fn set_active_token(&self, ticker: Option<String>) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());

        let new_active = if let Some(ticker) = ticker {
            let token = self
                .config
                .tokens
                .iter()
                .find(|t| t.ticker == ticker)
                .ok_or_else(|| {
                    SdkError::InvalidInput(format!(
                        "Stable balance ticker '{ticker}' not found in configured tokens"
                    ))
                })?;
            cache.save_stable_balance_active_ticker(&ticker).await?;
            Some(token.clone())
        } else {
            cache.delete_stable_balance_active_ticker().await?;
            None
        };

        if let Some(token) = &new_active {
            info!(
                "Stable balance active token changed to: {} ({})",
                token.ticker, token.token_identifier
            );
        } else {
            info!("Stable balance deactivated");
        }

        *self.active_token.write().await = new_active;

        // Clear cached effective values since limits may differ per token
        self.effective_values.clear().await;

        Ok(())
    }

    /// Resolves the initial active token from the local cache and config.
    ///
    /// Resolution order:
    /// 1. If a cached ticker exists and is in the tokens list → use it
    /// 2. If a cached ticker exists but is NOT in the tokens list → inactive
    /// 3. If no cache exists → use `default_active_ticker` from config
    async fn resolve_initial_token(
        config: &StableBalanceConfig,
        storage: &Arc<dyn Storage>,
    ) -> Option<StableBalanceToken> {
        let cache = ObjectCacheRepository::new(storage.clone());

        match cache.fetch_stable_balance_active_ticker().await {
            Ok(Some(cached_ticker)) => {
                // Cached ticker exists — validate against config
                let token = config.tokens.iter().find(|t| t.ticker == cached_ticker);
                if token.is_none() {
                    info!(
                        "Cached stable balance ticker '{cached_ticker}' not found in config, deactivating"
                    );
                }
                token.cloned()
            }
            Ok(None) => {
                // No cache — use default from config
                config
                    .default_active_ticker
                    .as_ref()
                    .and_then(|ticker| config.tokens.iter().find(|t| t.ticker == *ticker).cloned())
            }
            Err(e) => {
                warn!("Failed to read stable balance cache: {e:?}, deactivating");
                None
            }
        }
    }

    /// Gets or initializes the effective threshold and reserved sats for auto-conversion.
    ///
    /// Returns cached values if they exist and haven't expired. Otherwise, fetches
    /// conversion limits and computes:
    /// - Effective threshold: `max(user_threshold, min_from_amount)`
    /// - Effective reserved: user value if set, otherwise `min_from_amount`
    pub(super) async fn get_or_init_effective_values(
        &self,
        active_token_identifier: &str,
    ) -> Result<(u64, u64), ConversionError> {
        // Return cached values if not expired
        if let Some(values) = self.effective_values.get().await {
            return Ok((values.threshold, values.reserved));
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

        let (_, reserved) = self
            .get_or_init_effective_values(&active_token_identifier)
            .await?;
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
    fn should_trigger_per_receive(payment: &Payment) -> bool {
        payment.payment_type == PaymentType::Receive && payment.method != PaymentMethod::Token
    }
}

#[macros::async_trait]
impl EventListener for StableBalance {
    async fn on_event(&self, event: SdkEvent) {
        match event {
            // Sync completed → notify synced_notify and trigger batch auto-convert
            SdkEvent::Synced => {
                self.synced_notify.notify_one();
                self.trigger_auto_convert();
            }

            // Payment received → queue per-receive conversion if eligible
            SdkEvent::PaymentSucceeded { payment } => {
                if Self::should_trigger_per_receive(&payment) {
                    debug!("Queueing per-receive conversion for payment {}", payment.id);
                    self.queue_per_receive_convert(&payment.id);
                }
            }

            _ => {}
        }
    }
}
