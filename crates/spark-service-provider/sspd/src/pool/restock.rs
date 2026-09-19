use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::wakeup::Wakeup;

#[derive(Debug, Clone)]
pub enum PoolEvent {
    FundingBroadcast {
        txid: String,
        /// The funded trees' total value.
        total_sats: u64,
        /// Each funded tree's leaf denomination.
        denominations: Vec<u64>,
    },
    LeavesAvailable {
        deposit_address: String,
        /// Each added leaf's value.
        denominations: Vec<u64>,
    },
}

/// How many events a listener may fall behind before it misses some. Publishing
/// never waits for a listener.
const EVENT_CAPACITY: usize = 64;

/// Leaves requested on top of the pool's target, and a channel of pool events.
/// Requests are kept in memory only, so a restart forgets those not yet funded.
pub struct RestockService {
    requested: Mutex<HashMap<u64, u32>>,
    events: broadcast::Sender<PoolEvent>,
    replenish: Wakeup,
}

impl RestockService {
    #[must_use]
    pub fn new(replenish: Wakeup) -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            requested: Mutex::new(HashMap::new()),
            events,
            replenish,
        }
    }

    pub fn request(&self, denomination: u64, count: u32) {
        {
            let mut requested = self.lock();
            let entry = requested.entry(denomination).or_insert(0);
            *entry = entry.saturating_add(count);
        }
        self.replenish.wake();
    }

    pub fn fulfil(&self, funded: &HashMap<u64, u32>) {
        let mut requested = self.lock();
        for (denomination, count) in funded {
            if let Some(entry) = requested.get_mut(denomination) {
                *entry = entry.saturating_sub(*count);
            }
        }
        requested.retain(|_, count| *count > 0);
    }

    #[must_use]
    pub fn pending(&self) -> HashMap<u64, u32> {
        self.lock().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PoolEvent> {
        self.events.subscribe()
    }

    /// Having no listeners is not an error.
    pub fn publish(&self, event: PoolEvent) {
        let _ = self.events.send(event);
    }

    /// Recovers from poisoning: every update leaves the map consistent.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, u32>> {
        self.requested
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_accumulate_until_funded() {
        let service = RestockService::new(Wakeup::new());
        service.request(1024, 2);
        service.request(1024, 3);
        service.request(64, 1);
        assert_eq!(service.pending().get(&1024), Some(&5));

        service.fulfil(&HashMap::from([(1024, 4), (64, 1)]));
        assert_eq!(service.pending(), HashMap::from([(1024, 1)]));
    }
}
