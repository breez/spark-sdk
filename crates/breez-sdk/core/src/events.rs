use core::fmt;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use platform_utils::time::Instant;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tracing::info;
use uuid::Uuid;

use crate::{DepositInfo, LightningAddressInfo, Payment};

/// Events emitted by the SDK
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SdkEvent {
    /// Emitted when the wallet has been synchronized with the network
    Synced,
    /// Emitted when the SDK was unable to claim deposits
    UnclaimedDeposits {
        unclaimed_deposits: Vec<DepositInfo>,
    },
    ClaimedDeposits {
        claimed_deposits: Vec<DepositInfo>,
    },
    PaymentSucceeded {
        payment: Payment,
    },
    PaymentPending {
        payment: Payment,
    },
    PaymentFailed {
        payment: Payment,
    },
    Optimization {
        // Named with `optimization` prefix to avoid collision with `event` keyword in C#
        optimization_event: OptimizationEvent,
    },
    LightningAddressChanged {
        lightning_address: Option<LightningAddressInfo>,
    },
    NewDeposits {
        new_deposits: Vec<DepositInfo>,
    },
}

impl SdkEvent {
    pub(crate) fn from_payment(payment: Payment) -> Self {
        match payment.status {
            crate::PaymentStatus::Completed => SdkEvent::PaymentSucceeded { payment },
            crate::PaymentStatus::Pending => SdkEvent::PaymentPending { payment },
            crate::PaymentStatus::Failed => SdkEvent::PaymentFailed { payment },
        }
    }
}

impl fmt::Display for SdkEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdkEvent::Synced => write!(f, "Synced"),
            SdkEvent::UnclaimedDeposits { unclaimed_deposits } => {
                write!(f, "UnclaimedDeposits: {unclaimed_deposits:?}")
            }
            SdkEvent::ClaimedDeposits { claimed_deposits } => {
                write!(f, "ClaimedDeposits: {claimed_deposits:?}")
            }
            SdkEvent::PaymentSucceeded { payment } => {
                write!(f, "PaymentSucceeded: {payment:?}")
            }
            SdkEvent::PaymentPending { payment } => {
                write!(f, "PaymentPending: {payment:?}")
            }
            SdkEvent::PaymentFailed { payment } => {
                write!(f, "PaymentFailed: {payment:?}")
            }
            SdkEvent::Optimization {
                optimization_event: event,
            } => {
                write!(f, "Optimization: {event:?}")
            }
            SdkEvent::LightningAddressChanged { lightning_address } => {
                write!(f, "LightningAddressChanged: {lightning_address:?}")
            }
            SdkEvent::NewDeposits { new_deposits } => {
                write!(f, "NewDeposits: {new_deposits:?}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum OptimizationEvent {
    /// Optimization has started with the given number of rounds.
    Started { total_rounds: u32 },
    /// A round has completed.
    RoundCompleted {
        current_round: u32,
        total_rounds: u32,
    },
    /// Optimization completed successfully.
    Completed,
    /// Optimization was cancelled.
    Cancelled,
    /// Optimization failed with an error.
    Failed { error: String },
    /// Optimization was skipped because leaves are already optimal.
    Skipped,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub struct InternalSyncedEvent {
    pub wallet: bool,
    pub wallet_state: bool,
    pub deposits: bool,
    pub lnurl_metadata: bool,
    pub storage_incoming: Option<u32>,
}

impl InternalSyncedEvent {
    pub fn any(&self) -> bool {
        self.wallet
            || self.wallet_state
            || self.deposits
            || self.lnurl_metadata
            || self.storage_incoming.is_some()
    }

    pub fn any_non_zero(&self) -> bool {
        self.wallet
            || self.wallet_state
            || self.deposits
            || self.lnurl_metadata
            || self.storage_incoming.is_some_and(|v| v > 0)
    }

    pub fn merge(&self, other: &InternalSyncedEvent) -> Self {
        Self {
            wallet: self.wallet || other.wallet,
            wallet_state: self.wallet_state || other.wallet_state,
            deposits: self.deposits || other.deposits,
            lnurl_metadata: self.lnurl_metadata || other.lnurl_metadata,
            storage_incoming: self
                .storage_incoming
                .zip(other.storage_incoming)
                .map(|(a, b)| a.saturating_add(b))
                .or(self.storage_incoming)
                .or(other.storage_incoming),
        }
    }
}

/// Trait for event listeners
#[cfg_attr(feature = "uniffi", uniffi::export(callback_interface))]
#[macros::async_trait]
pub trait EventListener: Send + Sync {
    /// Called when an event occurs
    async fn on_event(&self, event: SdkEvent);
}

/// Middleware that can intercept and transform events before they reach external listeners.
///
/// Middleware processes events in a chain. Each middleware receives the event from the
/// previous one and can:
/// - Pass it through unchanged: `Some(event)`
/// - Transform it: `Some(modified_event)`
/// - Suppress it: `None`
#[macros::async_trait]
pub trait EventMiddleware: Send + Sync {
    /// Process an event. Return `Some` to forward (possibly modified), `None` to suppress.
    async fn process(&self, event: SdkEvent) -> Option<SdkEvent>;
}

/// Event publisher that manages event listeners and middleware.
///
/// Events flow through three phases:
/// 1. Internal listeners see raw events (SDK components like `wait_for_payment`)
/// 2. Middleware chain can transform or suppress events
/// 3. External listeners see processed events (client event handlers)
pub struct EventEmitter {
    has_real_time_sync: bool,
    rtsync_failed: AtomicBool,
    listener_index: AtomicU64,
    /// Internal listeners see ALL events before middleware processing
    internal_listeners: RwLock<BTreeMap<String, Box<dyn EventListener>>>,
    /// Middleware chain that can transform/suppress events
    middleware: RwLock<Vec<Box<dyn EventMiddleware>>>,
    /// External listeners see events after middleware processing
    external_listeners: RwLock<BTreeMap<String, Box<dyn EventListener>>>,
    synced_event_buffer: Mutex<Option<InternalSyncedEvent>>,
}

impl EventEmitter {
    /// Create a new event emitter
    pub fn new(has_real_time_sync: bool) -> Self {
        Self {
            has_real_time_sync,
            rtsync_failed: AtomicBool::new(false),
            listener_index: AtomicU64::new(0),
            internal_listeners: RwLock::new(BTreeMap::new()),
            middleware: RwLock::new(Vec::new()),
            external_listeners: RwLock::new(BTreeMap::new()),
            synced_event_buffer: Mutex::new(Some(InternalSyncedEvent::default())),
        }
    }

    /// Add an external listener to receive events
    ///
    /// # Arguments
    ///
    /// * `listener` - The listener to add
    ///
    /// # Returns
    ///
    /// A unique identifier for the listener, which can be used to remove it later
    pub async fn add_external_listener(&self, listener: Box<dyn EventListener>) -> String {
        let index = self.listener_index.fetch_add(1, Ordering::Relaxed);
        let id = format!("listener_{}-{}", index, Uuid::new_v4());
        let mut listeners = self.external_listeners.write().await;
        listeners.insert(id.clone(), listener);
        id
    }

    /// Remove an external listener by its ID
    ///
    /// # Arguments
    ///
    /// * `id` - The ID returned from `add_listener`
    ///
    /// # Returns
    ///
    /// `true` if the listener was found and removed, `false` otherwise
    pub async fn remove_external_listener(&self, id: &str) -> bool {
        let mut listeners = self.external_listeners.write().await;
        listeners.remove(id).is_some()
    }

    /// Add an internal listener that sees all raw events before middleware processing.
    ///
    /// Used by SDK components (e.g., `wait_for_payment`) that need to observe events
    /// that middleware may suppress.
    pub async fn add_internal_listener(&self, listener: Box<dyn EventListener>) -> String {
        let index = self.listener_index.fetch_add(1, Ordering::Relaxed);
        let id = format!("internal_{}-{}", index, Uuid::new_v4());
        let mut listeners = self.internal_listeners.write().await;
        listeners.insert(id.clone(), listener);
        id
    }

    /// Remove an internal listener by its ID
    pub async fn remove_internal_listener(&self, id: &str) -> bool {
        let mut listeners = self.internal_listeners.write().await;
        listeners.remove(id).is_some()
    }

    /// Add middleware to the event processing chain.
    ///
    /// Middleware can transform or suppress events before they reach external listeners.
    pub async fn add_middleware(&self, middleware: Box<dyn EventMiddleware>) {
        let mut mw = self.middleware.write().await;
        mw.push(middleware);
    }

    /// Emit an event through the three-phase pipeline:
    /// 1. Internal listeners see the raw event
    /// 2. Middleware chain can transform or suppress
    /// 3. External listeners see the processed event
    pub async fn emit(&self, event: &SdkEvent) {
        let start = Instant::now();
        let event_label = format!("{event}");
        let mut internal_total = std::time::Duration::ZERO;
        let mut middleware_total = std::time::Duration::ZERO;
        let mut external_total = std::time::Duration::ZERO;

        // Phase 1: Internal listeners see raw event
        let internal = self.internal_listeners.read().await;
        let internal_count = internal.len();
        for (id, listener) in internal.iter() {
            let t = Instant::now();
            listener.on_event(event.clone()).await;
            let dt = t.elapsed();
            internal_total = internal_total.saturating_add(dt);
            info!("emit({event_label}) internal listener {id}: {dt:?}");
        }
        drop(internal);

        // Phase 2: Middleware chain
        let mut event = Some(event.clone());
        let middleware = self.middleware.read().await;
        let middleware_count = middleware.len();
        for (i, mw) in middleware.iter().enumerate() {
            if let Some(e) = event {
                let t = Instant::now();
                event = mw.process(e).await;
                let dt = t.elapsed();
                middleware_total = middleware_total.saturating_add(dt);
                info!("emit({event_label}) middleware #{i}: {dt:?}");
            } else {
                break;
            }
        }
        drop(middleware);

        // Phase 3: External listeners see processed event
        let mut external_count = 0;
        if let Some(ref event) = event {
            let listeners = self.external_listeners.read().await;
            external_count = listeners.len();
            for (id, listener) in listeners.iter() {
                let t = Instant::now();
                listener.on_event(event.clone()).await;
                let dt = t.elapsed();
                external_total = external_total.saturating_add(dt);
                info!("emit({event_label}) external listener {id}: {dt:?}");
            }
        }

        info!(
            "emit({event_label}) completed in {:?} (internal[{}]={:?}, middleware[{}]={:?}, external[{}]={:?})",
            start.elapsed(),
            internal_count,
            internal_total,
            middleware_count,
            middleware_total,
            external_count,
            external_total
        );
    }

    pub async fn emit_synced(&self, synced: &InternalSyncedEvent) {
        if !synced.any() {
            // Nothing to emit
            return;
        }

        let mut mtx = self.synced_event_buffer.lock().await;

        let is_first_event = if let Some(buffered) = &*mtx {
            let merged = buffered.merge(synced);

            // The first synced event emitted should at least have the wallet synced.
            // Subsequent events might have only partial syncs.
            if merged.wallet
                && (!self.has_real_time_sync
                    || merged.storage_incoming.is_some()
                    || self.rtsync_failed.load(Ordering::Relaxed))
            {
                *mtx = None;
            } else {
                *mtx = Some(merged);
                return;
            }

            true
        } else {
            false
        };

        drop(mtx);

        // Only emit zero real-time syncs on the first event.
        if !is_first_event && !synced.any_non_zero() {
            return;
        }

        // Emit the merged event
        self.emit(&SdkEvent::Synced).await;
    }

    /// Notify that real-time sync has failed. If the first synced event is still
    /// buffered and the wallet has already synced, release it immediately instead
    /// of waiting for a remote pull that may never arrive.
    pub async fn notify_rtsync_failed(&self) {
        self.rtsync_failed.store(true, Ordering::Relaxed);

        let mut mtx = self.synced_event_buffer.lock().await;
        if let Some(buffered) = &*mtx
            && buffered.wallet
        {
            *mtx = None;
            drop(mtx);
            self.emit(&SdkEvent::Synced).await;
        }
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    struct TestListener {
        received: Arc<AtomicBool>,
    }

    #[macros::async_trait]
    impl EventListener for TestListener {
        async fn on_event(&self, _event: SdkEvent) {
            self.received.store(true, Ordering::Relaxed);
        }
    }

    #[async_test_all]
    async fn test_event_emission() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        // Create the listener with a shared reference to the atomic boolean
        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        let _ = emitter.add_external_listener(listener).await;

        let event = SdkEvent::Synced {};

        emitter.emit(&event).await;

        // Check if event was received using the shared reference
        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_remove_listener() {
        let emitter = EventEmitter::new(false);

        // Create shared atomic booleans to track event reception
        let received1 = Arc::new(AtomicBool::new(false));
        let received2 = Arc::new(AtomicBool::new(false));

        // Create listeners with their own shared references
        let listener1 = Box::new(TestListener {
            received: received1.clone(),
        });

        let listener2 = Box::new(TestListener {
            received: received2.clone(),
        });

        let id1 = emitter.add_external_listener(listener1).await;
        let id2 = emitter.add_external_listener(listener2).await;

        // Remove the first listener
        assert!(emitter.remove_external_listener(&id1).await);

        // Emit an event
        let event = SdkEvent::Synced {};
        emitter.emit(&event).await;

        // The first listener should not receive the event
        assert!(!received1.load(Ordering::Relaxed));

        // The second listener should receive the event
        assert!(received2.load(Ordering::Relaxed));

        // Remove the second listener
        assert!(emitter.remove_external_listener(&id2).await);

        // Try to remove a non-existent listener
        assert!(!emitter.remove_external_listener("non-existent-id").await);
    }

    #[async_test_all]
    async fn test_synced_event_only_emitted_with_wallet_sync() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Emit synced event without wallet sync - should NOT emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: true,
                deposits: true,
                lnurl_metadata: true,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        // Emit synced event with wallet sync - should emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: Some(1),
            })
            .await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_has_real_time_sync_synced_event_only_emitted_with_wallet_and_storage_sync() {
        let emitter = EventEmitter::new(true);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Emit synced event with storage
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: Some(0),
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        // Emit synced event with wallet sync - should emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_has_real_time_sync_synced_event_only_emitted_with_wallet_and_storage_sync_reverse()
     {
        let emitter = EventEmitter::new(true);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Emit synced event with wallet sync
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        // Emit synced event with storage - should emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: Some(0),
            })
            .await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_rtsync_failed_emits_synced_on_wallet_alone() {
        let emitter = EventEmitter::new(true);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Wallet synced but rtsync hasn't failed yet — should NOT emit
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        // rtsync fails — should immediately release the buffered event
        emitter.notify_rtsync_failed().await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_rtsync_failed_before_wallet_sync_emits_on_wallet() {
        let emitter = EventEmitter::new(true);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // rtsync fails before wallet syncs — nothing to release yet
        emitter.notify_rtsync_failed().await;

        assert!(!received.load(Ordering::Relaxed));

        // Wallet syncs — should emit immediately (rtsync already marked failed)
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_synced_event_buffers_until_wallet_sync() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Emit multiple partial syncs without wallet sync
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: true,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: true,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: true,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));

        // Finally emit wallet sync - should emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_synced_event_all_true() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Emit synced event with wallet and other components - should emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: true,
                deposits: true,
                lnurl_metadata: true,
                storage_incoming: Some(1),
            })
            .await;

        assert!(received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_synced_event_empty_does_not_emit() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_external_listener(listener).await;

        // Emit empty synced event - should NOT emit Synced
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert!(!received.load(Ordering::Relaxed));
    }

    #[async_test_all]
    async fn test_subsequent_syncs_after_wallet_emit_immediately() {
        use std::sync::atomic::AtomicUsize;

        struct CountingListener {
            count: Arc<AtomicUsize>,
        }

        #[macros::async_trait]
        impl EventListener for CountingListener {
            async fn on_event(&self, event: SdkEvent) {
                if matches!(event, SdkEvent::Synced) {
                    self.count.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let emitter = EventEmitter::new(true);
        let count = Arc::new(AtomicUsize::new(0));

        let listener = Box::new(CountingListener {
            count: count.clone(),
        });

        emitter.add_external_listener(listener).await;

        // First sync with wallet - should emit
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: Some(0),
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 1);

        // Subsequent partial sync without wallet - should emit (buffer cleared after first wallet sync)
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: true,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 2);

        // Another partial sync - should emit
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: true,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 3);

        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: true,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 4);

        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: Some(1),
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 5);

        // storage_incoming with Some(0) - should NOT emit after first sync
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: Some(0),
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 5);

        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 6);
    }

    // ── Helpers for middleware / internal listener tests ──

    /// Listener that records all received events
    struct RecordingListener {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingListener {
        fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
            let events = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    events: events.clone(),
                },
                events,
            )
        }
    }

    #[macros::async_trait]
    impl EventListener for RecordingListener {
        async fn on_event(&self, event: SdkEvent) {
            self.events.lock().await.push(format!("{event}"));
        }
    }

    /// Middleware that suppresses all Synced events
    struct SuppressSyncedMiddleware;

    #[macros::async_trait]
    impl EventMiddleware for SuppressSyncedMiddleware {
        async fn process(&self, event: SdkEvent) -> Option<SdkEvent> {
            match event {
                SdkEvent::Synced => None,
                other => Some(other),
            }
        }
    }

    /// Middleware that replaces `PaymentSucceeded` with `PaymentPending`
    struct DowngradePaymentMiddleware;

    #[macros::async_trait]
    impl EventMiddleware for DowngradePaymentMiddleware {
        async fn process(&self, event: SdkEvent) -> Option<SdkEvent> {
            match event {
                SdkEvent::PaymentSucceeded { payment } => {
                    Some(SdkEvent::PaymentPending { payment })
                }
                other => Some(other),
            }
        }
    }

    /// Middleware that suppresses all events
    struct SuppressAllMiddleware;

    #[macros::async_trait]
    impl EventMiddleware for SuppressAllMiddleware {
        async fn process(&self, _event: SdkEvent) -> Option<SdkEvent> {
            None
        }
    }

    fn test_payment() -> Payment {
        Payment {
            id: "test-id".to_string(),
            payment_type: crate::PaymentType::Receive,
            status: crate::PaymentStatus::Completed,
            amount: 1000,
            fees: 10,
            timestamp: 123_456,
            method: crate::PaymentMethod::Spark,
            details: None,
            conversion_details: None,
        }
    }

    // ── Internal listener tests ──

    #[async_test_all]
    async fn test_internal_listener_receives_events() {
        let emitter = EventEmitter::new(false);
        let (listener, events) = RecordingListener::new();

        emitter.add_internal_listener(Box::new(listener)).await;

        emitter.emit(&SdkEvent::Synced).await;

        let log = events.lock().await;
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("Synced"));
    }

    #[async_test_all]
    async fn test_remove_internal_listener() {
        let emitter = EventEmitter::new(false);
        let (listener, events) = RecordingListener::new();

        let id = emitter.add_internal_listener(Box::new(listener)).await;

        assert!(emitter.remove_internal_listener(&id).await);
        assert!(!emitter.remove_internal_listener(&id).await);

        emitter.emit(&SdkEvent::Synced).await;

        assert!(events.lock().await.is_empty());
    }

    // ── Middleware tests ──

    #[async_test_all]
    async fn test_middleware_suppresses_event_for_external() {
        let emitter = EventEmitter::new(false);
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_external_listener(Box::new(ext)).await;
        emitter
            .add_middleware(Box::new(SuppressSyncedMiddleware))
            .await;

        emitter.emit(&SdkEvent::Synced).await;

        // External should NOT see the suppressed event
        assert!(ext_events.lock().await.is_empty());
    }

    #[async_test_all]
    async fn test_middleware_transforms_event() {
        let emitter = EventEmitter::new(false);
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_external_listener(Box::new(ext)).await;
        emitter
            .add_middleware(Box::new(DowngradePaymentMiddleware))
            .await;

        let event = SdkEvent::PaymentSucceeded {
            payment: test_payment(),
        };
        emitter.emit(&event).await;

        let log = ext_events.lock().await;
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("PaymentPending"));
    }

    #[async_test_all]
    async fn test_middleware_passthrough_unmatched_events() {
        let emitter = EventEmitter::new(false);
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_external_listener(Box::new(ext)).await;
        emitter
            .add_middleware(Box::new(SuppressSyncedMiddleware))
            .await;

        // Synced is suppressed, PaymentSucceeded passes through
        emitter.emit(&SdkEvent::Synced).await;
        let event = SdkEvent::PaymentSucceeded {
            payment: test_payment(),
        };
        emitter.emit(&event).await;

        let log = ext_events.lock().await;
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("PaymentSucceeded"));
    }

    #[async_test_all]
    async fn test_middleware_chain_ordering() {
        let emitter = EventEmitter::new(false);
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_external_listener(Box::new(ext)).await;

        // First middleware transforms PaymentSucceeded → PaymentPending
        emitter
            .add_middleware(Box::new(DowngradePaymentMiddleware))
            .await;
        // Second middleware suppresses Synced (doesn't affect PaymentPending)
        emitter
            .add_middleware(Box::new(SuppressSyncedMiddleware))
            .await;

        let event = SdkEvent::PaymentSucceeded {
            payment: test_payment(),
        };
        emitter.emit(&event).await;

        let log = ext_events.lock().await;
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("PaymentPending"));
    }

    #[async_test_all]
    async fn test_suppress_all_middleware_stops_chain() {
        let emitter = EventEmitter::new(false);
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_external_listener(Box::new(ext)).await;

        // SuppressAll first — nothing should reach the next middleware or external
        emitter
            .add_middleware(Box::new(SuppressAllMiddleware))
            .await;
        emitter
            .add_middleware(Box::new(DowngradePaymentMiddleware))
            .await;

        emitter.emit(&SdkEvent::Synced).await;
        let event = SdkEvent::PaymentSucceeded {
            payment: test_payment(),
        };
        emitter.emit(&event).await;

        assert!(ext_events.lock().await.is_empty());
    }

    // ── Three-phase flow tests ──

    #[async_test_all]
    async fn test_three_phase_emit_flow() {
        let emitter = EventEmitter::new(false);
        let (int, int_events) = RecordingListener::new();
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_internal_listener(Box::new(int)).await;
        emitter.add_external_listener(Box::new(ext)).await;
        emitter
            .add_middleware(Box::new(SuppressSyncedMiddleware))
            .await;

        // Synced: internal sees it, middleware suppresses it, external doesn't
        emitter.emit(&SdkEvent::Synced).await;

        assert_eq!(int_events.lock().await.len(), 1);
        assert!(ext_events.lock().await.is_empty());

        // PaymentSucceeded: both see it (middleware passes it through)
        let event = SdkEvent::PaymentSucceeded {
            payment: test_payment(),
        };
        emitter.emit(&event).await;

        assert_eq!(int_events.lock().await.len(), 2);
        assert_eq!(ext_events.lock().await.len(), 1);
    }

    #[async_test_all]
    async fn test_internal_sees_raw_event_external_sees_transformed() {
        let emitter = EventEmitter::new(false);
        let (int, int_events) = RecordingListener::new();
        let (ext, ext_events) = RecordingListener::new();

        emitter.add_internal_listener(Box::new(int)).await;
        emitter.add_external_listener(Box::new(ext)).await;
        emitter
            .add_middleware(Box::new(DowngradePaymentMiddleware))
            .await;

        let event = SdkEvent::PaymentSucceeded {
            payment: test_payment(),
        };
        emitter.emit(&event).await;

        let int_log = int_events.lock().await;
        let ext_log = ext_events.lock().await;

        // Internal sees the original PaymentSucceeded
        assert_eq!(int_log.len(), 1);
        assert!(int_log[0].contains("PaymentSucceeded"));

        // External sees the transformed PaymentPending
        assert_eq!(ext_log.len(), 1);
        assert!(ext_log[0].contains("PaymentPending"));
    }

    #[async_test_all]
    async fn test_no_listeners_no_middleware_does_not_panic() {
        let emitter = EventEmitter::new(false);
        emitter.emit(&SdkEvent::Synced).await;
        // Should not panic
    }

    #[async_test_all]
    async fn test_empty_event_does_not_emit_after_wallet_sync() {
        use std::sync::atomic::AtomicUsize;

        struct CountingListener {
            count: Arc<AtomicUsize>,
        }

        #[macros::async_trait]
        impl EventListener for CountingListener {
            async fn on_event(&self, event: SdkEvent) {
                if matches!(event, SdkEvent::Synced) {
                    self.count.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let emitter = EventEmitter::new(false);
        let count = Arc::new(AtomicUsize::new(0));

        let listener = Box::new(CountingListener {
            count: count.clone(),
        });

        emitter.add_external_listener(listener).await;

        // First sync with wallet - should emit
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: true,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 1);

        // Empty sync after wallet sync - should NOT emit (all fields false)
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: false,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 1); // Count should remain 1

        // Another non-empty sync - should emit
        emitter
            .emit_synced(&InternalSyncedEvent {
                wallet: false,
                wallet_state: true,
                deposits: false,
                lnurl_metadata: false,
                storage_incoming: None,
            })
            .await;

        assert_eq!(count.load(Ordering::Relaxed), 2); // Now count should be 2
    }
}
