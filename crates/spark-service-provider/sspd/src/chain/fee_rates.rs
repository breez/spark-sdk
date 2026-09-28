use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::warn;

use crate::fees::{FeeRateSource, MIN_FEE_RATE_SAT_PER_KW};

use super::ChainClient;

/// Fees quoted seconds apart should agree, and the fee market does not move fast
/// enough for a fresher rate to be worth a round trip per quote.
const DEFAULT_TTL: Duration = Duration::from_secs(60);

/// Deep enough that the estimate is a market rate rather than the next-block
/// spike.
const DEFAULT_CONF_TARGET: u32 = 6;

/// How old the last rate can be and still stand in for a failed estimate: an
/// older one may no longer match the market.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

pub struct CachedFeeRates {
    chain: Arc<dyn ChainClient + Send + Sync>,
    conf_target: u32,
    ttl: Duration,
    cached: Mutex<Option<(Instant, u64)>>,
}

impl CachedFeeRates {
    pub fn new(chain: Arc<dyn ChainClient + Send + Sync>) -> Self {
        Self {
            chain,
            conf_target: DEFAULT_CONF_TARGET,
            ttl: DEFAULT_TTL,
            cached: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl FeeRateSource for CachedFeeRates {
    async fn sat_per_kw(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let mut cached = self.cached.lock().await;
        if let Some((fetched_at, rate)) = *cached
            && fetched_at.elapsed() < self.ttl
        {
            return Ok(rate);
        }
        match self.chain.estimate_fee_rate(self.conf_target).await {
            Ok(rate) => {
                let rate = rate.max(MIN_FEE_RATE_SAT_PER_KW);
                *cached = Some((Instant::now(), rate));
                Ok(rate)
            }
            Err(e) => match *cached {
                Some((fetched_at, rate)) if fetched_at.elapsed() < STALE_AFTER => {
                    warn!("fee rate estimate failed, using the last rate {rate} sat/kw: {e}");
                    Ok(rate)
                }
                _ => Err(format!("no fee rate available: {e}").into()),
            },
        }
    }
}
