use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bitcoin::FeeRate;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::fees::{FeeRateSource, MIN_FEE_RATE};

use super::{ChainClient, ChainError};

/// Fees quoted seconds apart should agree, and the fee market does not move fast
/// enough for a fresher rate to be worth a call to bitcoind.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Deep enough that the estimate is a market rate rather than the next-block
/// spike.
const DEFAULT_CONF_TARGET: u32 = 6;

/// How old the last rate can be while sspd still gives it out. An older rate
/// may no longer match the market.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// sspd refreshes the fee rate from bitcoind's estimate on a timer, so a caller
/// gets the rate without a call to bitcoind.
pub struct CachedFeeRates {
    chain: Arc<dyn ChainClient + Send + Sync>,
    conf_target: u32,
    cached: Mutex<Option<(Instant, FeeRate)>>,
}

impl CachedFeeRates {
    pub fn new(chain: Arc<dyn ChainClient + Send + Sync>) -> Self {
        Self {
            chain,
            conf_target: DEFAULT_CONF_TARGET,
            cached: Mutex::new(None),
        }
    }

    pub async fn refresh(&self) -> Result<(), ChainError> {
        let rate = self.chain.estimate_fee_rate(self.conf_target).await?;
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), rate.max(MIN_FEE_RATE)));
        Ok(())
    }

    pub async fn run(self: Arc<Self>, token: CancellationToken) {
        loop {
            tokio::select! {
                () = token.cancelled() => return,
                () = tokio::time::sleep(REFRESH_INTERVAL) => {}
            }
            if let Err(e) = self.refresh().await {
                warn!("could not get a fee rate estimate, keeping the last rate: {e}");
            }
        }
    }
}

#[async_trait::async_trait]
impl FeeRateSource for CachedFeeRates {
    async fn fee_rate(&self) -> Result<FeeRate, Box<dyn std::error::Error + Send + Sync>> {
        match *self.cached.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((fetched_at, rate)) if fetched_at.elapsed() < STALE_AFTER => Ok(rate),
            _ => Err("no fee rate available".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use bitcoin::{Block, BlockHash, OutPoint, Transaction};

    use super::*;
    use crate::chain::{BroadcastError, OutputSpend, SyncState};

    #[derive(Default)]
    struct Bitcoind {
        rate: AtomicU64,
        failing: AtomicBool,
        estimates: AtomicU64,
    }

    #[async_trait::async_trait]
    impl ChainClient for Bitcoind {
        async fn broadcast_tx(&self, _tx: Transaction) -> Result<(), BroadcastError> {
            unimplemented!()
        }

        async fn broadcast_package(&self, _txs: &[Transaction]) -> Result<(), BroadcastError> {
            unimplemented!()
        }

        async fn estimate_fee_rate(&self, _conf_target: u32) -> Result<FeeRate, ChainError> {
            self.estimates.fetch_add(1, Ordering::SeqCst);
            if self.failing.load(Ordering::SeqCst) {
                return Err(ChainError::General("bitcoind unavailable".into()));
            }
            Ok(FeeRate::from_sat_per_kwu(self.rate.load(Ordering::SeqCst)))
        }

        async fn sync_state(&self) -> Result<SyncState, ChainError> {
            unimplemented!()
        }

        async fn get_best_block_hash(&self) -> Result<BlockHash, ChainError> {
            unimplemented!()
        }

        async fn get_block_hash(&self, _height: u64) -> Result<Option<BlockHash>, ChainError> {
            unimplemented!()
        }

        async fn get_block(&self, _hash: &BlockHash) -> Result<Block, ChainError> {
            unimplemented!()
        }

        async fn get_spends(
            &self,
            _outpoints: &[OutPoint],
        ) -> Result<Vec<OutputSpend>, ChainError> {
            unimplemented!()
        }

        async fn wait_for_block_height(
            &self,
            _height: u64,
            _timeout: Duration,
        ) -> Result<(), ChainError> {
            unimplemented!()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_reads_the_last_estimate_without_a_call_to_bitcoind() {
        let bitcoind = Arc::new(Bitcoind::default());
        bitcoind.rate.store(1_000, Ordering::SeqCst);
        let rates = Arc::new(CachedFeeRates::new(Arc::clone(&bitcoind) as Arc<_>));
        assert!(rates.fee_rate().await.is_err(), "no estimate yet");

        rates.refresh().await.unwrap();
        for _ in 0..10 {
            assert_eq!(
                rates.fee_rate().await.unwrap(),
                FeeRate::from_sat_per_kwu(1_000)
            );
        }
        assert_eq!(bitcoind.estimates.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_timer_keeps_the_rate_fresh_and_a_stale_rate_is_not_given_out() {
        let bitcoind = Arc::new(Bitcoind::default());
        bitcoind.rate.store(1_000, Ordering::SeqCst);
        let rates = Arc::new(CachedFeeRates::new(Arc::clone(&bitcoind) as Arc<_>));
        rates.refresh().await.unwrap();
        let token = CancellationToken::new();
        let running = tokio::spawn(Arc::clone(&rates).run(token.clone()));

        bitcoind.rate.store(2_000, Ordering::SeqCst);
        tokio::time::sleep(REFRESH_INTERVAL.saturating_add(Duration::from_secs(1))).await;
        assert_eq!(
            rates.fee_rate().await.unwrap(),
            FeeRate::from_sat_per_kwu(2_000)
        );

        bitcoind.failing.store(true, Ordering::SeqCst);
        tokio::time::sleep(STALE_AFTER.saturating_sub(Duration::from_secs(120))).await;
        assert_eq!(
            rates.fee_rate().await.unwrap(),
            FeeRate::from_sat_per_kwu(2_000)
        );
        tokio::time::sleep(Duration::from_secs(180)).await;
        assert!(rates.fee_rate().await.is_err());

        token.cancel();
        running.await.unwrap();
    }

    #[tokio::test]
    async fn a_rate_below_the_floor_is_raised_to_it() {
        let bitcoind = Arc::new(Bitcoind::default());
        bitcoind.rate.store(1, Ordering::SeqCst);
        let rates = CachedFeeRates::new(Arc::clone(&bitcoind) as Arc<_>);
        rates.refresh().await.unwrap();
        assert_eq!(rates.fee_rate().await.unwrap(), MIN_FEE_RATE);
    }
}
