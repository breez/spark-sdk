use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub struct Shutdown {
    token: CancellationToken,
    failed: AtomicBool,
}

impl Shutdown {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            token: CancellationToken::new(),
            failed: AtomicBool::new(false),
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
        self.failed.store(true, Ordering::SeqCst);
        self.token.cancel();
    }

    #[must_use]
    pub fn is_failure(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
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
        assert!(!shutdown.is_failure());
    }

    #[test]
    fn a_subsystem_failure_is_carried_to_the_exit_status() {
        let shutdown = Shutdown::new();
        shutdown.fail("chain monitor", &"no route to bitcoind");
        assert!(shutdown.child().is_cancelled());
        assert!(shutdown.is_failure());
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
