use core::fmt;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{DepositInfo, Payment};

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
        }
    }
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

/// Event publisher that manages event listeners
pub struct EventEmitter {
    has_real_time_sync: bool,
    listener_index: AtomicU64,
    listeners: RwLock<BTreeMap<String, Box<dyn EventListener>>>,
    synced_event_buffer: Mutex<Option<InternalSyncedEvent>>,
}

impl EventEmitter {
    /// Create a new event emitter
    pub fn new(has_real_time_sync: bool) -> Self {
        Self {
            has_real_time_sync,
            listener_index: AtomicU64::new(0),
            listeners: RwLock::new(BTreeMap::new()),
            synced_event_buffer: Mutex::new(Some(InternalSyncedEvent::default())),
        }
    }

    /// Add a listener to receive events
    ///
    /// # Arguments
    ///
    /// * `listener` - The listener to add
    ///
    /// # Returns
    ///
    /// A unique identifier for the listener, which can be used to remove it later
    pub async fn add_listener(&self, listener: Box<dyn EventListener>) -> String {
        let index = self.listener_index.fetch_add(1, Ordering::Relaxed);
        let id = format!("listener_{}-{}", index, Uuid::new_v4());
        let mut listeners = self.listeners.write().await;
        listeners.insert(id.clone(), listener);
        id
    }

    /// Remove a listener by its ID
    ///
    /// # Arguments
    ///
    /// * `id` - The ID returned from `add_listener`
    ///
    /// # Returns
    ///
    /// `true` if the listener was found and removed, `false` otherwise
    pub async fn remove_listener(&self, id: &str) -> bool {
        let mut listeners = self.listeners.write().await;
        listeners.remove(id).is_some()
    }

    /// Emit an event to all registered listeners
    pub async fn emit(&self, event: &SdkEvent) {
        // Get a read lock on the listeners
        let listeners = self.listeners.read().await;

        // Emit the event to each listener
        for listener in listeners.values() {
            listener.on_event(event.clone()).await;
        }
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
            if merged.wallet && (!self.has_real_time_sync || merged.storage_incoming.is_some()) {
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

        let _ = emitter.add_listener(listener).await;

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

        let id1 = emitter.add_listener(listener1).await;
        let id2 = emitter.add_listener(listener2).await;

        // Remove the first listener
        assert!(emitter.remove_listener(&id1).await);

        // Emit an event
        let event = SdkEvent::Synced {};
        emitter.emit(&event).await;

        // The first listener should not receive the event
        assert!(!received1.load(Ordering::Relaxed));

        // The second listener should receive the event
        assert!(received2.load(Ordering::Relaxed));

        // Remove the second listener
        assert!(emitter.remove_listener(&id2).await);

        // Try to remove a non-existent listener
        assert!(!emitter.remove_listener("non-existent-id").await);
    }

    #[async_test_all]
    async fn test_synced_event_only_emitted_with_wallet_sync() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_listener(listener).await;

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

        emitter.add_listener(listener).await;

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

        emitter.add_listener(listener).await;

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
    async fn test_synced_event_buffers_until_wallet_sync() {
        let emitter = EventEmitter::new(false);
        let received = Arc::new(AtomicBool::new(false));

        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        emitter.add_listener(listener).await;

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

        emitter.add_listener(listener).await;

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

        emitter.add_listener(listener).await;

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

        emitter.add_listener(listener).await;

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

        emitter.add_listener(listener).await;

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
