use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// How long sspd still counts as synced after the chain monitor stopped
/// waiting for bitcoind's next block. The monitor does not wait while it stores
/// new blocks or while bitcoind returns errors. So sspd stays synced through a
/// few blocks in a row and through a short outage of bitcoind, but not through
/// a long outage.
const GRACE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Default)]
enum Waiting {
    #[default]
    Never,
    Now,
    Stopped(Instant),
}

/// sspd is synced while the chain monitor waits for bitcoind's next block,
/// with the record at bitcoind's tip and bitcoind reporting that it is synced.
/// sspd stays synced for [`GRACE`] after the monitor stops waiting.
#[derive(Clone, Default)]
pub struct Synced(Arc<Mutex<Waiting>>);

impl Synced {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn at_tip() -> Self {
        let synced = Self::default();
        synced.waiting();
        synced
    }

    fn set(&self, waiting: Waiting) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = waiting;
    }

    pub(super) fn waiting(&self) {
        self.set(Waiting::Now);
    }

    pub(super) fn stopped(&self) {
        let mut waiting = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*waiting, Waiting::Now) {
            *waiting = Waiting::Stopped(Instant::now());
        }
    }

    pub fn is_synced(&self) -> bool {
        match *self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Waiting::Never => false,
            Waiting::Now => true,
            Waiting::Stopped(at) => at.elapsed() < GRACE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn sspd_is_synced_while_the_monitor_waits_and_for_a_while_after() {
        let synced = Synced::new();
        assert!(!synced.is_synced());

        synced.waiting();
        tokio::time::advance(Duration::from_secs(3600)).await;
        assert!(synced.is_synced());

        synced.stopped();
        tokio::time::advance(GRACE.saturating_sub(Duration::from_secs(1))).await;
        assert!(synced.is_synced());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!synced.is_synced());

        synced.waiting();
        assert!(synced.is_synced());
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_that_ends_twice_counts_from_the_first_time() {
        let synced = Synced::new();
        synced.waiting();
        synced.stopped();
        tokio::time::advance(GRACE.saturating_sub(Duration::from_secs(1))).await;
        synced.stopped();
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!synced.is_synced());
    }
}
