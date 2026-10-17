//! The call queue makes the calls to bitcoind that sspd needs to serve users'
//! requests. It makes only a few of these calls at once, however many requests
//! users send, because sspd's own loops need bitcoind too.

use std::sync::Arc;
use std::time::Duration;

use bitcoin::OutPoint;
use tokio::sync::{Semaphore, SemaphorePermit};

use super::{ChainError, OutputSpend, UnspentOutput};

const SLOTS: usize = 2;

/// How long sspd waits for the calls a user's request needs before it fails
/// the request.
pub const TIMEOUT: Duration = Duration::from_secs(10);

#[async_trait::async_trait]
pub trait RequestChainClient: Send + Sync {
    /// Returns the output at `outpoint` while it is unspent, counting the
    /// mempool.
    async fn unspent_output(
        &self,
        outpoint: &OutPoint,
    ) -> Result<Option<UnspentOutput>, ChainError>;

    /// Returns the spend of each of `outpoints` that is an input of a
    /// transaction in the chain or in the mempool.
    async fn get_spends(&self, outpoints: &[OutPoint]) -> Result<Vec<OutputSpend>, ChainError>;
}

/// The call queue makes each call once one of its slots is free, in the order
/// it got the calls. A caller that stops waiting leaves the queue.
pub struct CallQueue {
    bitcoind: Arc<dyn RequestChainClient>,
    slots: Semaphore,
}

impl CallQueue {
    pub fn new(bitcoind: Arc<dyn RequestChainClient>) -> Self {
        Self {
            bitcoind,
            slots: Semaphore::new(SLOTS),
        }
    }

    async fn slot(&self) -> Result<SemaphorePermit<'_>, ChainError> {
        self.slots
            .acquire()
            .await
            .map_err(|e| ChainError::General(Box::new(e)))
    }

    pub async fn unspent_output(
        &self,
        outpoint: &OutPoint,
    ) -> Result<Option<UnspentOutput>, ChainError> {
        let _slot = self.slot().await?;
        self.bitcoind.unspent_output(outpoint).await
    }

    pub async fn get_spends(&self, outpoints: &[OutPoint]) -> Result<Vec<OutputSpend>, ChainError> {
        let _slot = self.slot().await?;
        self.bitcoind.get_spends(outpoints).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bitcoin::Txid;
    use bitcoin::hashes::Hash;
    use tokio::sync::Notify;

    use super::*;

    #[derive(Default)]
    struct Bitcoind {
        release: Notify,
        in_flight: AtomicUsize,
        most_in_flight: AtomicUsize,
        order: Mutex<Vec<u32>>,
    }

    #[async_trait::async_trait]
    impl RequestChainClient for Bitcoind {
        async fn unspent_output(
            &self,
            outpoint: &OutPoint,
        ) -> Result<Option<UnspentOutput>, ChainError> {
            self.order.lock().unwrap().push(outpoint.vout);
            let in_flight = self
                .in_flight
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            self.most_in_flight.fetch_max(in_flight, Ordering::SeqCst);
            self.release.notified().await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn get_spends(
            &self,
            _outpoints: &[OutPoint],
        ) -> Result<Vec<OutputSpend>, ChainError> {
            unimplemented!()
        }
    }

    fn output(index: u32) -> OutPoint {
        OutPoint::new(Txid::all_zeros(), index)
    }

    async fn let_tasks_run() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn calls_take_a_slot_each_in_the_order_they_came() {
        let bitcoind = Arc::new(Bitcoind::default());
        let call_queue = Arc::new(CallQueue::new(
            Arc::clone(&bitcoind) as Arc<dyn RequestChainClient>
        ));
        let mut calls = Vec::new();
        for index in 0..5 {
            let call_queue = Arc::clone(&call_queue);
            calls.push(tokio::spawn(async move {
                call_queue.unspent_output(&output(index)).await
            }));
            let_tasks_run().await;
        }
        assert_eq!(bitcoind.in_flight.load(Ordering::SeqCst), SLOTS);

        for _ in 0..5 {
            bitcoind.release.notify_one();
            let_tasks_run().await;
        }
        for call in calls {
            call.await.unwrap().unwrap();
        }
        assert_eq!(bitcoind.most_in_flight.load(Ordering::SeqCst), SLOTS);
        assert_eq!(*bitcoind.order.lock().unwrap(), [0, 1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn a_caller_that_stops_waiting_leaves_the_queue() {
        let bitcoind = Arc::new(Bitcoind::default());
        let call_queue = Arc::new(CallQueue::new(
            Arc::clone(&bitcoind) as Arc<dyn RequestChainClient>
        ));
        let mut calls = Vec::new();
        for index in 0..SLOTS {
            let call_queue = Arc::clone(&call_queue);
            let index = u32::try_from(index).unwrap();
            calls.push(tokio::spawn(async move {
                call_queue.unspent_output(&output(index)).await
            }));
        }
        let_tasks_run().await;

        let timed_out = tokio::time::timeout(
            Duration::from_millis(20),
            call_queue.unspent_output(&output(7)),
        )
        .await;
        assert!(timed_out.is_err());

        for _ in 0..SLOTS {
            bitcoind.release.notify_one();
        }
        for call in calls {
            call.await.unwrap().unwrap();
        }
        assert!(!bitcoind.order.lock().unwrap().contains(&7));
        assert_eq!(call_queue.slots.available_permits(), SLOTS);
    }
}
