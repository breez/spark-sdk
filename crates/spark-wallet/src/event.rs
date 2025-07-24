use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc::Sender;
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::WalletEvent;

pub(super) struct EventManager {
    cancel: broadcast::Sender<()>,
    listeners: Arc<Mutex<HashMap<Uuid, Sender<WalletEvent>>>>,
}

impl EventManager {
    pub fn new() -> Self {
        Self {
            cancel: broadcast::channel(1).0,
            listeners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn add_listener(&self, listener: Sender<WalletEvent>) {
        let id = Uuid::now_v7();
        tracing::debug!("Adding listener with ID: {}", id);

        let clone = listener.clone();
        self.listeners.lock().await.insert(id, listener);
        let listeners = Arc::clone(&self.listeners);

        // This cancel token is dropped when the eventmanager itself is dropped.
        let mut cancel = self.cancel.subscribe();
        tokio::spawn(async move {
            tokio::select! {
                _ = clone.closed() => {}
                _ = cancel.recv() => {
                    // Exit if the cancel token is triggered
                    return;
                }
            }
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
