use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub struct Shutdown {
    token: CancellationToken,
    failure: Mutex<Option<String>>,
}

impl Shutdown {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            token: CancellationToken::new(),
            failure: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn child(&self) -> CancellationToken {
        self.token.child_token()
    }

    pub fn stop(&self, reason: &str) {
        info!("shutting down: {reason}");
        self.token.cancel();
    }

    pub fn fail(&self, subsystem: &str, error: &dyn Display) {
        error!("shutting down: {subsystem} failed: {error}");
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| format!("{subsystem} failed: {error}"));
        self.token.cancel();
    }

    /// The failure that brought the daemon down: the first, since the rest
    /// usually follow from it.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requested_stop_is_not_a_failure() {
        let shutdown = Shutdown::new();
        shutdown.stop("shutdown signal");
        assert!(shutdown.child().is_cancelled());
        assert_eq!(shutdown.failure(), None);
    }

    #[test]
    fn a_subsystem_failure_is_carried_to_the_exit_status() {
        let shutdown = Shutdown::new();
        shutdown.fail("chain monitor", &"no route to bitcoind");
        shutdown.fail("GraphQL API server", &"cancelled");
        assert!(shutdown.child().is_cancelled());
        assert_eq!(
            shutdown.failure().as_deref(),
            Some("chain monitor failed: no route to bitcoind")
        );
    }

    #[test]
    fn a_task_stopping_itself_leaves_the_daemon_running() {
        let shutdown = Shutdown::new();
        let task = shutdown.child();
        task.cancel();
        assert!(task.is_cancelled());
        assert!(
            !shutdown.child().is_cancelled(),
            "one task stopping must not bring the daemon down"
        );
    }
}
