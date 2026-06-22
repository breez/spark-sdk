//! Pluggable store for the timestamps used in Turnkey activity submissions.
//!
//! Turnkey fingerprints the whole submitted body, `timestampMs` included, so an
//! activity that needs approval only folds into the same activity if it is
//! re-submitted byte-for-byte. This store records the timestamp chosen the first
//! time an activity (identified by a hash of its content) is submitted; a later
//! submission of the same activity reuses it and resolves to the existing
//! (possibly already-approved) activity instead of creating a new one.

use std::collections::HashMap;
use std::sync::Mutex;

/// Records and returns the `timestampMs` to stamp a Turnkey activity with, keyed
/// by a hash of its content.
#[macros::async_trait]
pub trait TurnkeyActivityStore: Send + Sync {
    /// Returns the timestamp (ms since epoch) for the activity identified by
    /// `key`. The first call for a given `key` records and returns
    /// `fallback_now_ms`; later calls return that same recorded value.
    async fn timestamp_ms(&self, key: &str, fallback_now_ms: u64) -> u64;
}

/// Process-local [`TurnkeyActivityStore`]. Enough when the approval-trigger
/// submission and the later re-submission share a process; bridging separate
/// processes needs a persistent implementation injected via the signer builder.
#[derive(Default)]
pub struct InMemoryTurnkeyActivityStore {
    timestamps: Mutex<HashMap<String, u64>>,
}

#[macros::async_trait]
impl TurnkeyActivityStore for InMemoryTurnkeyActivityStore {
    async fn timestamp_ms(&self, key: &str, fallback_now_ms: u64) -> u64 {
        let mut timestamps = self
            .timestamps
            .lock()
            .expect("turnkey activity store mutex poisoned");
        *timestamps.entry(key.to_string()).or_insert(fallback_now_ms)
    }
}
