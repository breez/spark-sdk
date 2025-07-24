use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use crate::WalletEvent;

pub(super) struct EventManager {
    listeners: Arc<Mutex<HashMap<Uuid, Sender<WalletEvent>>>>,
}

impl EventManager {
    pub fn new() -> Self {
        Self {
            listeners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn add_listener(&self, listener: Sender<WalletEvent>) {
        let id = Uuid::now_v7();
        tracing::debug!("Adding listener with ID: {}", id);

        let clone = listener.clone();
        self.listeners.lock().await.insert(id, listener);
        let listeners = Arc::clone(&self.listeners);
        tokio::spawn(async move {
            // TODO: Add cancellation logic, because this will run until the receiver is dropped.
            clone.closed().await;
            tracing::debug!(
                "Removing listener with ID '{}' because receiver dropped.",
                id
            );
            listeners.lock().await.remove(&id);
        });
    }

    pub async fn notify_listeners(&self, event: WalletEvent) {
        let listeners = self.listeners.lock().await;
        for (id, listener) in listeners.iter() {
            tracing::debug!("Notifying listener with ID: {}", id);
            if let Err(e) = listener.send(event.clone()).await {
                tracing::error!("Failed to send event to listener with ID '{}': {}", id, e);
            }
        }
    }
}
