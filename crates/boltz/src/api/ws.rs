use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite_wasm::{Message, WebSocketStream};

use crate::error::BoltzError;

use super::types::{WsMessage, WsSubscribeMessage};

/// Swap status update dispatched from the WebSocket.
#[derive(Debug, Clone)]
pub struct SwapStatusUpdate {
    pub swap_id: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub transaction: Option<super::types::SwapTransaction>,
}

/// Commands sent from `subscribe()`/`unsubscribe()` to the reader loop.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
enum ReaderCommand {
    Subscribe(String),
    Shutdown,
}

/// WebSocket subscriber for Boltz swap status updates.
pub struct SwapStatusSubscriber {
    senders: Arc<Mutex<HashMap<String, mpsc::Sender<SwapStatusUpdate>>>>,
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    ws_url: String,
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    cmd_tx: mpsc::Sender<ReaderCommand>,
}

impl SwapStatusSubscriber {
    // ─── Native ──────────────────────────────────────────────────────

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    #[expect(clippy::unused_async)]
    pub async fn connect(ws_url: &str) -> Result<Self, BoltzError> {
        let senders: Arc<Mutex<HashMap<String, mpsc::Sender<SwapStatusUpdate>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (cmd_tx, cmd_rx) = mpsc::channel(16);

        let reader_handle =
            tokio::spawn(Self::reader_loop(ws_url.to_string(), senders.clone(), cmd_rx));

        Ok(Self {
            senders,
            reader_handle: Mutex::new(Some(reader_handle)),
            cmd_tx,
        })
    }

    // ─── WASM ────────────────────────────────────────────────────────

    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    #[expect(clippy::unused_async)]
    pub async fn connect(ws_url: &str) -> Result<Self, BoltzError> {
        Ok(Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
            ws_url: ws_url.to_string(),
        })
    }

    // ─── Common API ──────────────────────────────────────────────────

    pub async fn subscribe(
        &self,
        swap_id: &str,
    ) -> Result<mpsc::Receiver<SwapStatusUpdate>, BoltzError> {
        let (tx, rx) = mpsc::channel(32);
        self.senders
            .lock()
            .await
            .insert(swap_id.to_string(), tx);

        // Tell the reader loop to send a subscribe message for this new ID
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        {
            let _ = self.cmd_tx.send(ReaderCommand::Subscribe(swap_id.to_string())).await;
        }

        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        {
            let ws_url = self.ws_url.clone();
            let senders = self.senders.clone();
            let swap_id_owned = swap_id.to_string();
            wasm_bindgen_futures::spawn_local(async move {
                Self::single_swap_reader(ws_url, senders, swap_id_owned).await;
            });
        }

        tracing::info!(swap_id, "Subscribed to swap status updates");
        Ok(rx)
    }

    pub async fn unsubscribe(&self, swap_id: &str) {
        self.senders.lock().await.remove(swap_id);
        tracing::info!(swap_id, "Unsubscribed from swap status updates");
    }

    pub async fn close(&self) {
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        {
            let _ = self.cmd_tx.send(ReaderCommand::Shutdown).await;
            if let Some(handle) = self.reader_handle.lock().await.take() {
                handle.abort();
            }
        }
        self.senders.lock().await.clear();
        tracing::info!("WebSocket subscriber closed");
    }

    // ─── Native reader loop ──────────────────────────────────────────

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    async fn reader_loop(
        ws_url: String,
        senders: Arc<Mutex<HashMap<String, mpsc::Sender<SwapStatusUpdate>>>>,
        mut cmd_rx: mpsc::Receiver<ReaderCommand>,
    ) {
        loop {
            let ws_stream = match Self::try_connect(&ws_url).await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::warn!("WebSocket connection failed: {e}, retrying in 5s");
                    tokio::select! {
                        () = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                        cmd = cmd_rx.recv() => {
                            if matches!(cmd, None | Some(ReaderCommand::Shutdown)) {
                                return;
                            }
                            continue; // Retry connect with new subscription
                        }
                    }
                }
            };

            tracing::info!("WebSocket connected to {ws_url}");
            let (mut write, mut read) = ws_stream.split();

            // Re-subscribe all currently-tracked swap IDs (needed after reconnect).
            // Drain any pending Subscribe commands first to avoid double-subscribing.
            let mut ids: Vec<String> = senders.lock().await.keys().cloned().collect();
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    ReaderCommand::Subscribe(id) => {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    ReaderCommand::Shutdown => return,
                }
            }
            if !ids.is_empty() {
                let msg = WsSubscribeMessage::subscribe(ids);
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = write.send(Message::Text(json.into())).await;
                }
            }

            // Read loop — also listens for new subscribe commands
            let should_shutdown = loop {
                tokio::select! {
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                Self::handle_message(&text, &senders).await;
                            }
                            Some(Ok(Message::Binary(data))) => {
                                if let Ok(text) = String::from_utf8(data.to_vec()) {
                                    Self::handle_message(&text, &senders).await;
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
                                // Send subscribe for the new ID immediately
                                let msg = WsSubscribeMessage::subscribe(vec![id]);
                                if let Ok(json) = serde_json::to_string(&msg)
                                    && let Err(e) = write.send(Message::Text(json.into())).await
                                {
                                    tracing::warn!("Failed to send subscribe: {e}");
                                    break false; // Reconnect
                                }
                            }
                            Some(ReaderCommand::Shutdown) | None => break true,
                        }
                    }
                }
            };

            if should_shutdown {
                return;
            }

            // Wait before reconnecting
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                cmd = cmd_rx.recv() => {
                    if matches!(cmd, None | Some(ReaderCommand::Shutdown)) {
                        return;
                    }
                }
            }
        }
    }

    // ─── WASM inline reader ──────────────────────────────────────────

    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    async fn single_swap_reader(
        ws_url: String,
        senders: Arc<Mutex<HashMap<String, mpsc::Sender<SwapStatusUpdate>>>>,
        swap_id: String,
    ) {
        let ws_stream = match Self::try_connect(&ws_url).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("WASM WS connection failed: {e}");
                return;
            }
        };

        let (mut write, mut read) = ws_stream.split();

        let msg = WsSubscribeMessage::subscribe(vec![swap_id.clone()]);
        if let Ok(json) = serde_json::to_string(&msg)
            && let Err(e) = write.send(Message::Text(json.into())).await
        {
            tracing::warn!("Failed to send subscribe: {e}");
            return;
        }

        while let Some(Ok(Message::Text(text))) = read.next().await {
            Self::handle_message(&text, &senders).await;
            if !senders.lock().await.contains_key(&swap_id) {
                break;
            }
        }
    }

    // ─── Shared helpers ──────────────────────────────────────────────

    async fn try_connect(url: &str) -> Result<WebSocketStream, BoltzError> {
        tokio_tungstenite_wasm::connect(url)
            .await
            .map_err(|e| BoltzError::WebSocket(format!("Connection failed: {e}")))
    }

    async fn handle_message(
        text: &str,
        senders: &Arc<Mutex<HashMap<String, mpsc::Sender<SwapStatusUpdate>>>>,
    ) {
        let msg: WsMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("Failed to parse WS message: {e}");
                return;
            }
        };

        // Discard ping/pong events
        if let Some(ref event) = msg.event
            && (event == "ping" || event == "pong")
        {
            return;
        }

        // Only process swap.update channel messages
        if msg.channel.as_deref() != Some("swap.update") {
            return;
        }

        // Process swap updates
        if let Some(args) = msg.args {
            let senders = senders.lock().await;
            for update in args {
                let status_update = SwapStatusUpdate {
                    swap_id: update.id.clone(),
                    status: update.status,
                    failure_reason: update.failure_reason,
                    transaction: update.transaction,
                };

                if let Some(sender) = senders.get(&update.id) {
                    if sender.send(status_update).await.is_err() {
                        tracing::debug!(
                            swap_id = update.id,
                            "Receiver dropped for swap, update discarded"
                        );
                    }
                } else {
                    tracing::debug!(
                        swap_id = update.id,
                        "No subscriber for swap update"
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
        let senders = Arc::new(Mutex::new(HashMap::new()));
        SwapStatusSubscriber::handle_message(r#"{"event":"ping"}"#, &senders).await;
        SwapStatusSubscriber::handle_message(r#"{"event":"pong"}"#, &senders).await;
    }

    #[tokio::test]
    async fn test_handle_message_dispatches_update() {
        let senders = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::channel(32);
        senders.lock().await.insert("swap123".to_string(), tx);

        let msg = r#"{
            "event": "update",
            "channel": "swap.update",
            "args": [{
                "id": "swap123",
                "status": "transaction.confirmed",
                "transaction": { "id": "0xabc", "hex": "0xdef" }
            }]
        }"#;

        SwapStatusSubscriber::handle_message(msg, &senders).await;

        let update = rx.recv().await.unwrap();
        assert_eq!(update.swap_id, "swap123");
        assert_eq!(update.status, "transaction.confirmed");
        assert!(update.transaction.is_some());
    }

    #[tokio::test]
    async fn test_handle_message_unknown_swap_ignored() {
        let senders = Arc::new(Mutex::new(HashMap::new()));

        let msg = r#"{
            "channel": "swap.update",
            "args": [{
                "id": "unknown_swap",
                "status": "transaction.mempool"
            }]
        }"#;

        SwapStatusSubscriber::handle_message(msg, &senders).await;
    }

    #[tokio::test]
    async fn test_handle_message_wrong_channel_ignored() {
        let senders = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::channel(32);
        senders.lock().await.insert("swap1".to_string(), tx);

        let msg = r#"{
            "channel": "some.other.channel",
            "args": [{
                "id": "swap1",
                "status": "transaction.confirmed"
            }]
        }"#;

        SwapStatusSubscriber::handle_message(msg, &senders).await;
        assert!(rx.try_recv().is_err());
    }
}
