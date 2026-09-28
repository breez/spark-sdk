use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use futures::{FutureExt, future::BoxFuture};

use testcontainers::core::logs::{LogFrame, consumer::LogConsumer};

/// A consumer that logs the output of container with the [`log`] crate.
///
/// By default, both standard out and standard error will both be emitted at INFO level.
#[derive(Debug)]
pub struct TracingConsumer {
    prefix: String,
    recent: Option<RecentLines>,
}

impl TracingConsumer {
    /// Creates a new instance of the logging consumer.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            recent: None,
        }
    }

    /// Keeps the standard out lines it does not log in `recent`.
    #[must_use]
    pub fn keeping(self, recent: RecentLines) -> Self {
        Self {
            recent: Some(recent),
            ..self
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
                    let text = String::from_utf8_lossy(bytes);
                    let message = self.format_message(&text);
                    // Only log stdout if SPARK_ITEST_VERBOSE is set
                    if std::env::var("SPARK_ITEST_VERBOSE").is_ok() {
                        tracing::info!("{message}");
                    } else if let Some(recent) = &self.recent {
                        recent.push(message.into_owned());
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

/// The latest lines a container wrote, for a failing test to print.
#[derive(Clone, Debug, Default)]
pub struct RecentLines(Arc<Mutex<VecDeque<String>>>);

impl RecentLines {
    const LIMIT: usize = 2_000;

    fn push(&self, line: String) {
        let mut lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() == Self::LIMIT {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    pub fn take(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_lines_keep_the_latest() {
        let recent = RecentLines::default();
        for n in 0..=RecentLines::LIMIT {
            recent.push(n.to_string());
        }
        let lines = recent.take();
        assert_eq!(lines.len(), RecentLines::LIMIT);
        assert_eq!(lines.first().map(String::as_str), Some("1"));
        assert!(recent.take().is_empty());
    }
}
