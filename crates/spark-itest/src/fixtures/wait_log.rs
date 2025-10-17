use futures::{FutureExt, future::BoxFuture};
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use testcontainers::core::logs::{LogFrame, consumer::LogConsumer};
use tokio::sync::oneshot;

// Type definition to simplify the complex nested type
type PatternSender = Arc<Mutex<Option<(String, oneshot::Sender<()>)>>>;

/// A consumer that monitors logs for specific patterns and signals when they're found
#[derive(Debug, Clone)]
pub struct WaitForLogConsumer {
    prefix: String,
    startup_complete_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    startup_pattern: &'static str,
    log_buffer: Arc<Mutex<Vec<String>>>,
    // Add a channel for custom pattern matching
    custom_pattern_tx: PatternSender,
}

impl WaitForLogConsumer {
    /// Creates a new instance of the waiting log consumer.
    pub fn new(
        prefix: impl Into<String>,
        startup_pattern: &'static str,
        startup_complete_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            startup_complete_tx: Arc::new(Mutex::new(Some(startup_complete_tx))),
            startup_pattern,
            log_buffer: Arc::new(Mutex::new(Vec::new())),
            custom_pattern_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Set a custom pattern to wait for, along with a channel to signal when it's found
    pub fn set_custom_pattern(&self, pattern: String, tx: oneshot::Sender<()>) {
        if let Ok(mut custom_tx) = self.custom_pattern_tx.lock() {
            *custom_tx = Some((pattern, tx));
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
            && let Some(tx) = tx_lock.take()
        {
            let _ = tx.send(());
            tracing::info!("[{}] Detected startup complete", self.prefix);
        }

        // Check for custom pattern
        if let Ok(mut custom_tx_lock) = self.custom_pattern_tx.lock()
            && let Some((pattern, tx)) = custom_tx_lock.take()
        {
            if message.contains(&pattern) {
                let _ = tx.send(());
                tracing::info!("[{}] Detected custom pattern: {}", self.prefix, pattern);
            } else {
                // Put it back if not found
                *custom_tx_lock = Some((pattern, tx));
            }
        }
    }

    /// Check the log buffer for a specific pattern
    pub fn check_log_buffer(&self, pattern: &str) -> bool {
        if let Ok(buffer) = self.log_buffer.lock() {
            buffer.iter().any(|message| message.contains(pattern))
        } else {
            false
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
