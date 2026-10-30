use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use super::{BlockHeader, ChainTip, SyncState};

/// How long sspd still counts as synced after the chain monitor stopped
/// waiting for bitcoind's next block. The monitor does not wait while it stores
/// new blocks or while bitcoind returns errors. So sspd stays synced through a
/// few blocks in a row and through a short outage of bitcoind, but not through
/// a long outage.
const GRACE: Duration = Duration::from_mins(1);

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

    pub fn chain_sync(&self, chain_tip: &ChainTip) -> ChainSync {
        match chain_tip.record() {
            Some(tip) if self.is_synced() => ChainSync::Synced { tip },
            record => ChainSync::NotSynced {
                record,
                bitcoind: chain_tip.bitcoind_sync_state(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChainSync {
    Synced {
        tip: BlockHeader,
    },
    NotSynced {
        /// The chain record's tip. `None` while the record has no block.
        record: Option<BlockHeader>,
        /// bitcoind's sync state at the start of the chain monitor's last sync.
        /// `None` until the monitor has fetched it since sspd started.
        bitcoind: Option<SyncState>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn sspd_is_synced_while_the_monitor_waits_and_for_a_while_after() {
        let synced = Synced::new();
        assert!(!synced.is_synced());

        synced.waiting();
        tokio::time::advance(Duration::from_hours(1)).await;
        assert!(synced.is_synced());

        synced.stopped();
        tokio::time::advance(GRACE.saturating_sub(Duration::from_secs(1))).await;
        assert!(synced.is_synced());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!synced.is_synced());

        synced.waiting();
        assert!(synced.is_synced());
    }

    fn block(height: u64) -> BlockHeader {
        BlockHeader {
            hash: bitcoin::hashes::Hash::from_byte_array([7; 32]),
            height,
        }
    }

    #[test]
    fn before_the_monitor_has_reached_bitcoind_sspd_reports_what_it_has() {
        let synced = Synced::new();
        let nothing = ChainSync::NotSynced {
            record: None,
            bitcoind: None,
        };
        assert_eq!(synced.chain_sync(&ChainTip::default()), nothing);

        // After a restart, sspd has the stored tip before it has bitcoind's sync
        // state.
        let chain_tip = ChainTip::at(block(100));
        let stored = ChainSync::NotSynced {
            record: Some(block(100)),
            bitcoind: None,
        };
        assert_eq!(synced.chain_sync(&chain_tip), stored);

        let bitcoind = SyncState {
            height: 250,
            synced: false,
        };
        chain_tip.set_bitcoind_sync_state(bitcoind);
        let behind = ChainSync::NotSynced {
            record: Some(block(100)),
            bitcoind: Some(bitcoind),
        };
        assert_eq!(synced.chain_sync(&chain_tip), behind);
    }

    #[tokio::test(start_paused = true)]
    async fn a_synced_sspd_reports_the_chain_records_tip_until_it_is_no_longer_synced() {
        let synced = Synced::new();
        let chain_tip = ChainTip::at(block(100));
        let bitcoind = SyncState {
            height: 100,
            synced: true,
        };
        chain_tip.set_bitcoind_sync_state(bitcoind);

        synced.waiting();
        let at_tip = ChainSync::Synced { tip: block(100) };
        assert_eq!(synced.chain_sync(&chain_tip), at_tip);

        synced.stopped();
        tokio::time::advance(GRACE).await;
        let lost = ChainSync::NotSynced {
            record: Some(block(100)),
            bitcoind: Some(bitcoind),
        };
        assert_eq!(synced.chain_sync(&chain_tip), lost);
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
