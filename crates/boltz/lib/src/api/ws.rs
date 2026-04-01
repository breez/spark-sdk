use std::collections::HashSet;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use platform_utils::tokio;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite_wasm::{Message, WebSocketStream};

use crate::error::BoltzError;

use super::types::{WsMessage, WsSubscribeMessage};

/// Keep-alive ping interval to prevent idle disconnects.
const KEEP_ALIVE_INTERVAL: platform_utils::time::Duration =
    platform_utils::time::Duration::from_secs(15);

/// Delay between reconnection attempts.
const RECONNECT_DELAY: platform_utils::time::Duration =
    platform_utils::time::Duration::from_secs(5);

/// JSON-encoded ping message for the Boltz WS protocol.
const PING_JSON: &str = r#"{"op":"ping"}"#;

/// Swap status update dispatched from the WebSocket.
#[derive(Debug, Clone)]
pub struct SwapStatusUpdate {
    pub swap_id: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub transaction: Option<super::types::SwapTransaction>,
}

/// Commands sent to the reader loop.
enum ReaderCommand {
    Subscribe(String),
    Unsubscribe(String),
    Shutdown,
}

/// WebSocket subscriber for Boltz swap status updates.
///
/// All updates are dispatched through a single global channel (provided at
/// construction). Callers use `subscribe()`/`unsubscribe()` to control which
/// swap IDs the subscriber tracks; status updates for all tracked swaps flow
/// through the same channel.
pub struct SwapStatusSubscriber {
    /// IDs currently subscribed on the WS. Also used for resubscription on
    /// reconnect.
    subscribed_ids: Arc<Mutex<HashSet<String>>>,
    cmd_tx: mpsc::Sender<ReaderCommand>,
    /// Sync-safe handle used by `Drop` to abort the reader task if `close()`
    /// was never called.
    abort_handle: tokio::task::AbortHandle,
}

impl SwapStatusSubscriber {
    #[expect(clippy::unused_async)]
    pub async fn connect(
        ws_url: &str,
        global_tx: mpsc::Sender<SwapStatusUpdate>,
    ) -> Result<Self, BoltzError> {
        let subscribed_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let (cmd_tx, cmd_rx) = mpsc::channel(16);

        let reader_handle = tokio::spawn(Self::reader_loop(ws_url.to_string(), global_tx, cmd_rx));
        let abort_handle = reader_handle.abort_handle();

        Ok(Self {
            subscribed_ids,
            cmd_tx,
            abort_handle,
        })
    }

    /// Start tracking a swap ID. Status updates will be sent through the
    /// global channel provided at construction.
    pub async fn subscribe(&self, swap_id: &str) -> Result<(), BoltzError> {
        self.subscribed_ids.lock().await.insert(swap_id.to_string());

        let _ = self
            .cmd_tx
            .send(ReaderCommand::Subscribe(swap_id.to_string()))
            .await;

        tracing::info!(swap_id, "Subscribed to swap status updates");
        Ok(())
    }

    /// Stop tracking a swap ID.
    pub async fn unsubscribe(&self, swap_id: &str) {
        self.subscribed_ids.lock().await.remove(swap_id);

        let _ = self
            .cmd_tx
            .send(ReaderCommand::Unsubscribe(swap_id.to_string()))
            .await;

        tracing::info!(swap_id, "Unsubscribed from swap status updates");
    }

    pub async fn close(&self) {
        let _ = self.cmd_tx.send(ReaderCommand::Shutdown).await;
        self.subscribed_ids.lock().await.clear();
        tracing::info!("WebSocket subscriber closed");
    }
}

impl Drop for SwapStatusSubscriber {
    fn drop(&mut self) {
        self.abort_handle.abort();
    }
}

impl SwapStatusSubscriber {
    async fn reader_loop(
        ws_url: String,
        global_tx: mpsc::Sender<SwapStatusUpdate>,
        mut cmd_rx: mpsc::Receiver<ReaderCommand>,
    ) {
        // Track subscribed IDs locally in the loop for WS (re)subscription
        // messages. The authoritative set is `subscribed_ids` on the struct,
        // but we need a local copy to avoid holding the lock during I/O.
        let mut local_ids: HashSet<String> = HashSet::new();

        loop {
            let ws_stream = match Self::try_connect(&ws_url).await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::warn!("WebSocket connection failed: {e}, retrying in 5s");
                    tokio::select! {
                        () = tokio::time::sleep(RECONNECT_DELAY) => continue,
                        cmd = cmd_rx.recv() => {
                            match cmd {
                                Some(ReaderCommand::Subscribe(id)) => { local_ids.insert(id); }
                                Some(ReaderCommand::Unsubscribe(id)) => { local_ids.remove(&id); }
                                Some(ReaderCommand::Shutdown) | None => return,
                            }
                            continue;
                        }
                    }
                }
            };

            tracing::info!("WebSocket connected to {ws_url}");
            let (mut write, mut read) = ws_stream.split();

            // Drain pending commands before resubscribing.
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    ReaderCommand::Subscribe(id) => {
                        local_ids.insert(id);
                    }
                    ReaderCommand::Unsubscribe(id) => {
                        local_ids.remove(&id);
                    }
                    ReaderCommand::Shutdown => return,
                }
            }

            // Re-subscribe all tracked IDs after (re)connect.
            if !local_ids.is_empty() {
                let ids: Vec<String> = local_ids.iter().cloned().collect();
                let msg = WsSubscribeMessage::subscribe(ids);
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = write.send(Message::Text(json.into())).await;
                }
            }

            // Read loop — also listens for new commands and sends keep-alive pings.
            let should_shutdown = loop {
                tokio::select! {
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                Self::handle_message(&text, &global_tx).await;
                            }
                            Some(Ok(Message::Binary(data))) => {
                                if let Ok(text) = String::from_utf8(data.to_vec()) {
                                    Self::handle_message(&text, &global_tx).await;
                                }
                            }
                            Some(Ok(Message::Close(_)) | Err(_)) | None => {
                                tracing::info!("WebSocket disconnected, reconnecting");
                                break false;
                            }
                        }
                    }
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(ReaderCommand::Subscribe(id)) => {
                                local_ids.insert(id.clone());
                                let msg = WsSubscribeMessage::subscribe(vec![id]);
                                if let Ok(json) = serde_json::to_string(&msg)
                                    && let Err(e) = write.send(Message::Text(json.into())).await
                                {
                                    tracing::warn!("Failed to send subscribe: {e}");
                                    break false; // Reconnect
                                }
                            }
                            Some(ReaderCommand::Unsubscribe(id)) => {
                                local_ids.remove(&id);
                                // No need to send an unsubscribe to Boltz WS —
                                // we simply stop caring about updates for this ID.
                            }
                            Some(ReaderCommand::Shutdown) | None => break true,
                        }
                    }
                    () = tokio::time::sleep(KEEP_ALIVE_INTERVAL) => {
                        if let Err(e) = write.send(Message::Text(PING_JSON.into())).await {
                            tracing::warn!("Failed to send keep-alive ping: {e}");
                            break false; // Reconnect
                        }
                    }
                }
            };

            if should_shutdown {
                return;
            }

            // Wait before reconnecting.
            tokio::select! {
                () = tokio::time::sleep(RECONNECT_DELAY) => {}
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(ReaderCommand::Subscribe(id)) => { local_ids.insert(id); }
                        Some(ReaderCommand::Unsubscribe(id)) => { local_ids.remove(&id); }
                        Some(ReaderCommand::Shutdown) | None => return,
                    }
                }
            }
        }
    }

    // ─── Shared helpers ──────────────────────────────────────────────

    async fn try_connect(url: &str) -> Result<WebSocketStream, BoltzError> {
        tokio_tungstenite_wasm::connect(url)
            .await
            .map_err(|e| BoltzError::WebSocket(format!("Connection failed: {e}")))
    }

    async fn handle_message(text: &str, global_tx: &mpsc::Sender<SwapStatusUpdate>) {
        let msg: WsMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("Failed to parse WS message: {e}");
                return;
            }
        };

        if let Some(ref event) = msg.event
            && (event == "ping" || event == "pong")
        {
            return;
        }

        if msg.channel.as_deref() != Some("swap.update") {
            return;
        }

        if let Some(args) = msg.args {
            for update in args {
                let status_update = SwapStatusUpdate {
                    swap_id: update.id.clone(),
                    status: update.status,
                    failure_reason: update.failure_reason,
                    transaction: update.transaction,
                };

                if global_tx.send(status_update).await.is_err() {
                    tracing::debug!(
                        swap_id = update.id,
                        "Global receiver dropped, update discarded"
                    );
                }
            }
        }
    }
}

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use super::*;

    #[test]
    fn test_swap_status_update_clone() {
        let update = SwapStatusUpdate {
            swap_id: "test".to_string(),
            status: "transaction.confirmed".to_string(),
            failure_reason: None,
            transaction: None,
        };
        let cloned = update.clone();
        assert_eq!(cloned.swap_id, "test");
        assert_eq!(cloned.status, "transaction.confirmed");
    }

    #[tokio::test]
    async fn test_handle_message_ping_ignored() {
        let (tx, _rx) = mpsc::channel(32);
        SwapStatusSubscriber::handle_message(r#"{"event":"ping"}"#, &tx).await;
        SwapStatusSubscriber::handle_message(r#"{"event":"pong"}"#, &tx).await;
    }

    #[tokio::test]
    async fn test_handle_message_dispatches_update() {
        let (tx, mut rx) = mpsc::channel(32);

        let msg = r#"{
            "event": "update",
            "channel": "swap.update",
            "args": [{
                "id": "swap123",
                "status": "transaction.confirmed",
                "transaction": { "id": "0xabc", "hex": "0xdef" }
            }]
        }"#;

        SwapStatusSubscriber::handle_message(msg, &tx).await;

        let update = rx.recv().await.unwrap();
        assert_eq!(update.swap_id, "swap123");
        assert_eq!(update.status, "transaction.confirmed");
        assert!(update.transaction.is_some());
    }

    #[tokio::test]
    async fn test_handle_message_wrong_channel_ignored() {
        let (tx, mut rx) = mpsc::channel(32);

        let msg = r#"{
            "channel": "some.other.channel",
            "args": [{
                "id": "swap1",
                "status": "transaction.confirmed"
            }]
        }"#;

        SwapStatusSubscriber::handle_message(msg, &tx).await;
        assert!(rx.try_recv().is_err());
    }
}
