use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use platform_utils::tokio::sync::RwLock;

use crate::models::BoltzSwap;

/// Event emitted when a swap's state changes.
#[derive(Debug, Clone)]
pub enum BoltzSwapEvent {
    /// A swap's persisted state was updated.
    SwapUpdated { swap: BoltzSwap },
}

/// Callback trait for receiving swap events.
#[macros::async_trait]
pub trait BoltzEventListener: Send + Sync {
    async fn on_event(&self, event: BoltzSwapEvent);
}

/// Manages event listeners and broadcasts events.
pub struct EventEmitter {
    listener_index: AtomicU64,
    listeners: RwLock<BTreeMap<String, Box<dyn BoltzEventListener>>>,
}

impl EventEmitter {
    pub fn new() -> Self {
        Self {
            listener_index: AtomicU64::new(0),
            listeners: RwLock::new(BTreeMap::new()),
        }
    }

    pub async fn add_listener(&self, listener: Box<dyn BoltzEventListener>) -> String {
        let index = self.listener_index.fetch_add(1, Ordering::Relaxed);
        let id = format!("boltz_listener_{index}");
        self.listeners.write().await.insert(id.clone(), listener);
        id
    }

    pub async fn remove_listener(&self, id: &str) -> bool {
        self.listeners.write().await.remove(id).is_some()
    }

    /// Broadcast an event to all registered listeners.
    ///
    /// Holds a read lock for the duration of all `on_event` calls, so
    /// listeners should be fast (e.g. send on a channel). A slow listener
    /// blocks delivery to subsequent listeners and prevents
    /// `add_listener`/`remove_listener` from proceeding.
    pub async fn emit(&self, event: &BoltzSwapEvent) {
        let listeners = self.listeners.read().await;
        for listener in listeners.values() {
            listener.on_event(event.clone()).await;
        }
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}
