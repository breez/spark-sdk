use futures::{FutureExt, future::BoxFuture};
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use testcontainers::core::logs::{LogFrame, consumer::LogConsumer};
use tokio::sync::oneshot;

/// A consumer that monitors logs for specific patterns and signals when they're found
#[derive(Debug)]
pub struct WaitForLogConsumer {
    prefix: String,
    startup_complete_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    server_ready_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    startup_pattern: &'static str,
    server_pattern: &'static str,
    log_buffer: Arc<Mutex<Vec<String>>>,
}

impl WaitForLogConsumer {
    /// Creates a new instance of the waiting log consumer.
    pub fn new(
        prefix: impl Into<String>,
        startup_pattern: &'static str,
        server_pattern: &'static str,
        startup_complete_tx: oneshot::Sender<()>,
        server_ready_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            startup_complete_tx: Arc::new(Mutex::new(Some(startup_complete_tx))),
            server_ready_tx: Arc::new(Mutex::new(Some(server_ready_tx))),
            startup_pattern,
            server_pattern,
            log_buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn format_message<'a>(&self, message: &'a str) -> Cow<'a, str> {
        // Remove trailing newlines
        let message = message.trim_end_matches(['\n', '\r']);

        Cow::Owned(format!("[{}] {}", self.prefix, message))
    }

    fn check_and_signal(&self, message: &str) {
        // Store message in buffer
        if let Ok(mut buffer) = self.log_buffer.lock() {
            buffer.push(message.to_string());
        }

        // Check for startup complete message
        if message.contains(self.startup_pattern)
            && let Ok(mut tx_lock) = self.startup_complete_tx.lock()
                && let Some(tx) = tx_lock.take() {
                    let _ = tx.send(());
                    tracing::info!("[{}] Detected startup complete", self.prefix);
                }

        // Check for server ready message
        if message.contains(self.server_pattern)
            && let Ok(mut tx_lock) = self.server_ready_tx.lock()
                && let Some(tx) = tx_lock.take() {
                    let _ = tx.send(());
                    tracing::info!("[{}] Detected server ready", self.prefix);
                }
    }
}

impl LogConsumer for WaitForLogConsumer {
    fn accept<'a>(&'a self, record: &'a LogFrame) -> BoxFuture<'a, ()> {
        async move {
            match record {
                LogFrame::StdOut(bytes) => {
                    let message = String::from_utf8_lossy(bytes);
                    tracing::info!("{}", self.format_message(&message));
                    self.check_and_signal(&message);
                }
                LogFrame::StdErr(bytes) => {
                    let message = String::from_utf8_lossy(bytes);
                    tracing::info!("{}", self.format_message(&message));
                    self.check_and_signal(&message);
                }
            }
        }
        .boxed()
    }
}
