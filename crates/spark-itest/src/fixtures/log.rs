use std::borrow::Cow;

use futures::{FutureExt, future::BoxFuture};

use testcontainers::core::logs::{LogFrame, consumer::LogConsumer};

/// A consumer that logs the output of container with the [`log`] crate.
///
/// By default, both standard out and standard error will both be emitted at INFO level.
#[derive(Debug)]
pub struct TracingConsumer {
    prefix: String,
}

impl TracingConsumer {
    /// Creates a new instance of the logging consumer.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    fn format_message<'a>(&self, message: &'a str) -> Cow<'a, str> {
        // Remove trailing newlines
        let message = message.trim_end_matches(['\n', '\r']);

        Cow::Owned(format!("[{}] {}", self.prefix, message))
    }
}

impl Default for TracingConsumer {
    fn default() -> Self {
        Self::new("")
    }
}

impl LogConsumer for TracingConsumer {
    fn accept<'a>(&'a self, record: &'a LogFrame) -> BoxFuture<'a, ()> {
        async move {
            match record {
                LogFrame::StdOut(bytes) => {
                    // Only log stdout if SPARK_ITEST_VERBOSE is set
                    if std::env::var("SPARK_ITEST_VERBOSE").is_ok() {
                        tracing::info!("{}", self.format_message(&String::from_utf8_lossy(bytes)));
                    }
                }
                LogFrame::StdErr(bytes) => {
                    // Always log stderr (errors/warnings)
                    tracing::warn!("{}", self.format_message(&String::from_utf8_lossy(bytes)));
                }
            }
        }
        .boxed()
    }
}
