use core::fmt;
use std::{collections::HashMap, sync::RwLock};

use serde::Serialize;
use uuid::Uuid;

use crate::{DepositInfo, Payment};

/// Events emitted by the SDK
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SdkEvent {
    /// Emitted when the wallet has been synchronized with the network
    Synced,
    /// Emitted when the wallet failed to claim some deposits
    ClaimDepositsFailed {
        unclaimed_deposits: Vec<DepositInfo>,
    },
    ClaimDepositsSucceeded {
        claimed_deposits: Vec<DepositInfo>,
    },
    PaymentSucceeded {
        payment: Payment,
    },
}

impl fmt::Display for SdkEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdkEvent::Synced => write!(f, "Synced"),
            SdkEvent::ClaimDepositsFailed { unclaimed_deposits } => {
                write!(f, "ClaimDepositsFailed: {unclaimed_deposits:?}")
            }
            SdkEvent::ClaimDepositsSucceeded { claimed_deposits } => {
                write!(f, "ClaimDepositsSucceeded: {claimed_deposits:?}")
            }
            SdkEvent::PaymentSucceeded { payment } => {
                write!(f, "PaymentSucceeded: {payment:?}")
            }
        }
    }
}

/// Trait for event listeners
#[cfg_attr(feature = "uniffi", uniffi::export(callback_interface))]
pub trait EventListener: Send + Sync {
    /// Called when an event occurs
    fn on_event(&self, event: SdkEvent);
}

/// Event publisher that manages event listeners
pub struct EventEmitter {
    listeners: RwLock<HashMap<String, Box<dyn EventListener>>>,
}

impl EventEmitter {
    /// Create a new event emitter
    pub fn new() -> Self {
        Self {
            listeners: RwLock::new(HashMap::new()),
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
    pub fn add_listener(&self, listener: Box<dyn EventListener>) -> String {
        let id = Uuid::new_v4().to_string();
        let mut listeners = self.listeners.write().unwrap();
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
    pub fn remove_listener(&self, id: &str) -> bool {
        let mut listeners = self.listeners.write().unwrap();
        listeners.remove(id).is_some()
    }

    /// Emit an event to all registered listeners
    pub fn emit(&self, event: &SdkEvent) {
        // Get a read lock on the listeners
        let listeners = self.listeners.read().unwrap();

        // Emit the event to each listener
        for listener in listeners.values() {
            listener.on_event(event.clone());
        }
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    struct TestListener {
        received: Arc<AtomicBool>,
    }

    impl EventListener for TestListener {
        fn on_event(&self, _event: SdkEvent) {
            self.received.store(true, Ordering::Relaxed);
        }
    }

    #[test_all]
    fn test_event_emission() {
        let emitter = EventEmitter::new();
        let received = Arc::new(AtomicBool::new(false));

        // Create the listener with a shared reference to the atomic boolean
        let listener = Box::new(TestListener {
            received: received.clone(),
        });

        let _ = emitter.add_listener(listener);

        let event = SdkEvent::Synced {};

        emitter.emit(&event);

        // Check if event was received using the shared reference
        assert!(received.load(Ordering::Relaxed));
    }

    #[test_all]
    fn test_remove_listener() {
        let emitter = EventEmitter::new();

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

        let id1 = emitter.add_listener(listener1);
        let id2 = emitter.add_listener(listener2);

        // Remove the first listener
        assert!(emitter.remove_listener(&id1));

        // Emit an event
        let event = SdkEvent::Synced {};
        emitter.emit(&event);

        // The first listener should not receive the event
        assert!(!received1.load(Ordering::Relaxed));

        // The second listener should receive the event
        assert!(received2.load(Ordering::Relaxed));

        // Remove the second listener
        assert!(emitter.remove_listener(&id2));

        // Try to remove a non-existent listener
        assert!(!emitter.remove_listener("non-existent-id"));
    }
}
