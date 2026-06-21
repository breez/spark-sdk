//! TTL + single-flight wrapper around [`FiatService`].
//!
//! Cross-chain prepare issues 2-3 rate fetches per user action; this wrapper
//! collapses them into one cached call per TTL window. Concurrent cold callers
//! single-flight: while one fetch is in flight, others block on the `Mutex`
//! and pick up the populated cache. Entries are JSON-encoded under a string
//! key so any `Serialize + DeserializeOwned` response works.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use breez_sdk_common::{
    error::ServiceConnectivityError,
    fiat::{FiatCurrency, FiatService, Rate},
};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::Mutex;
use tracing::trace;

/// Default cache TTL. Long enough to amortize repeated fetches in a session,
/// short enough to bound fiat-rate drift between estimate and quote.
pub(crate) const DEFAULT_FIAT_CACHE_TTL: Duration = Duration::from_mins(1);

const RATES_KEY: &str = "fiat_rates";
const CURRENCIES_KEY: &str = "fiat_currencies";

/// [`FiatService`] wrapper with TTL caching and single-flight semantics.
pub(crate) struct CachedFiatService {
    inner: Arc<dyn FiatService>,
    ttl_ms: u128,
    cache: Mutex<HashMap<&'static str, CachedEntry>>,
}

struct CachedEntry {
    data: String,
    expires_at_ms: u128,
}

impl CachedFiatService {
    pub(crate) fn new(inner: Arc<dyn FiatService>, ttl: Duration) -> Self {
        Self {
            inner,
            ttl_ms: ttl.as_millis(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Look up `key` or invoke `fetch` to populate it. Single-flight: the lock
    /// is intentionally held across `fetch().await` so concurrent cold callers
    /// serialize and the second arrival reads the fresh cache instead of
    /// re-fetching. Do not release between check and fetch — that's a TOCTOU
    /// and defeats single-flight.
    async fn get_or_fetch<F, Fut, T>(
        &self,
        key: &'static str,
        fetch: F,
    ) -> Result<T, ServiceConnectivityError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ServiceConnectivityError>>,
        T: Serialize + DeserializeOwned,
    {
        let mut cache = self.cache.lock().await;
        let now = now_ms();
        if let Some(entry) = cache.get(key)
            && entry.expires_at_ms > now
        {
            trace!("CachedFiatService: cache hit for {key}");
            return serde_json::from_str(&entry.data).map_err(|e| {
                ServiceConnectivityError::Json(format!(
                    "CachedFiatService: decode failed for {key}: {e}"
                ))
            });
        }
        trace!("CachedFiatService: cache miss for {key}, fetching upstream");
        let response = fetch().await?;
        let data = serde_json::to_string(&response).map_err(|e| {
            ServiceConnectivityError::Json(format!(
                "CachedFiatService: encode failed for {key}: {e}"
            ))
        })?;
        cache.insert(
            key,
            CachedEntry {
                data,
                expires_at_ms: now.saturating_add(self.ttl_ms),
            },
        );
        Ok(response)
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[macros::async_trait]
impl FiatService for CachedFiatService {
    async fn fetch_fiat_currencies(&self) -> Result<Vec<FiatCurrency>, ServiceConnectivityError> {
        self.get_or_fetch(CURRENCIES_KEY, || self.inner.fetch_fiat_currencies())
            .await
    }

    async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError> {
        self.get_or_fetch(RATES_KEY, || self.inner.fetch_fiat_rates())
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use breez_sdk_common::fiat::Rate;
    use platform_utils::tokio;

    use super::*;

    /// Mock that counts how many times each method is invoked. Returns a
    /// configurable rate set on `fetch_fiat_rates`, or an error if armed.
    struct MockFiat {
        rates_calls: AtomicUsize,
        currencies_calls: AtomicUsize,
        rate_value: f64,
        fail_with: Option<fn() -> ServiceConnectivityError>,
        delay: Option<Duration>,
    }

    impl MockFiat {
        fn ok(rate: f64) -> Self {
            Self {
                rates_calls: AtomicUsize::new(0),
                currencies_calls: AtomicUsize::new(0),
                rate_value: rate,
                fail_with: None,
                delay: None,
            }
        }

        fn with_delay(rate: f64, delay: Duration) -> Self {
            Self {
                rates_calls: AtomicUsize::new(0),
                currencies_calls: AtomicUsize::new(0),
                rate_value: rate,
                fail_with: None,
                delay: Some(delay),
            }
        }

        fn failing() -> Self {
            Self {
                rates_calls: AtomicUsize::new(0),
                currencies_calls: AtomicUsize::new(0),
                rate_value: 0.0,
                fail_with: Some(|| ServiceConnectivityError::Other("upstream down".to_string())),
                delay: None,
            }
        }

        fn rates_calls(&self) -> usize {
            self.rates_calls.load(Ordering::SeqCst)
        }
        fn currencies_calls(&self) -> usize {
            self.currencies_calls.load(Ordering::SeqCst)
        }
    }

    #[macros::async_trait]
    impl FiatService for MockFiat {
        async fn fetch_fiat_currencies(
            &self,
        ) -> Result<Vec<FiatCurrency>, ServiceConnectivityError> {
            self.currencies_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }

        async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError> {
            self.rates_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if let Some(make_err) = self.fail_with {
                return Err(make_err());
            }
            Ok(vec![Rate {
                coin: "USD".to_string(),
                value: self.rate_value,
            }])
        }
    }

    fn usd_rate(rates: &[Rate]) -> f64 {
        rates
            .iter()
            .find(|r| r.coin == "USD")
            .map(|r| r.value)
            .unwrap_or_default()
    }

    /// Two sequential calls within TTL → inner called exactly once.
    #[macros::async_test_all]
    async fn warm_cache_avoids_redundant_fetch() {
        let mock = Arc::new(MockFiat::ok(60_000.0));
        let cached = CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            DEFAULT_FIAT_CACHE_TTL,
        );

        let r1 = cached.fetch_fiat_rates().await.unwrap();
        let r2 = cached.fetch_fiat_rates().await.unwrap();

        assert!((usd_rate(&r1) - 60_000.0).abs() < f64::EPSILON);
        assert!((usd_rate(&r2) - 60_000.0).abs() < f64::EPSILON);
        assert_eq!(
            mock.rates_calls(),
            1,
            "second call should be served from the cache"
        );
    }

    /// Currencies are cached on the same key strategy.
    #[macros::async_test_all]
    async fn warm_cache_applies_to_currencies_too() {
        let mock = Arc::new(MockFiat::ok(60_000.0));
        let cached = CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            DEFAULT_FIAT_CACHE_TTL,
        );

        cached.fetch_fiat_currencies().await.unwrap();
        cached.fetch_fiat_currencies().await.unwrap();
        assert_eq!(mock.currencies_calls(), 1);
    }

    /// Rates and currencies are independent cache entries — populating one
    /// doesn't satisfy the other.
    #[macros::async_test_all]
    async fn cache_entries_are_keyed_independently() {
        let mock = Arc::new(MockFiat::ok(60_000.0));
        let cached = CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            DEFAULT_FIAT_CACHE_TTL,
        );

        cached.fetch_fiat_rates().await.unwrap();
        cached.fetch_fiat_currencies().await.unwrap();
        assert_eq!(mock.rates_calls(), 1);
        assert_eq!(mock.currencies_calls(), 1);
    }

    /// A second call after the TTL elapses re-fetches.
    #[macros::async_test_all]
    async fn cache_refreshes_after_ttl() {
        let mock = Arc::new(MockFiat::ok(60_000.0));
        let cached = CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            Duration::from_millis(10),
        );

        cached.fetch_fiat_rates().await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        cached.fetch_fiat_rates().await.unwrap();

        assert_eq!(
            mock.rates_calls(),
            2,
            "expired cache should trigger refetch"
        );
    }

    /// N concurrent cold calls → inner called exactly once (single-flight).
    #[macros::async_test_all]
    async fn concurrent_cold_callers_single_flight() {
        let mock = Arc::new(MockFiat::with_delay(60_000.0, Duration::from_millis(50)));
        let cached = Arc::new(CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            DEFAULT_FIAT_CACHE_TTL,
        ));

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let cached = Arc::clone(&cached);
                tokio::spawn(async move { cached.fetch_fiat_rates().await.unwrap() })
            })
            .collect();

        for h in handles {
            let r = h.await.unwrap();
            assert!((usd_rate(&r) - 60_000.0).abs() < f64::EPSILON);
        }
        assert_eq!(
            mock.rates_calls(),
            1,
            "5 concurrent cold callers should produce exactly 1 upstream fetch"
        );
    }

    /// One caller hits a slow cold fetch; a second caller fires a follow-up
    /// while the first is still in flight. The second must NOT issue its own
    /// upstream call — it should serialize behind the first via the
    /// lock-across-await pattern and return the freshly-populated cache.
    ///
    /// This cements the deliberate `lock().await` → `fetch().await` design
    /// against future "optimizations" that would release the lock between
    /// the cache check and the upstream call (which would TOCTOU and defeat
    /// single-flight).
    #[macros::async_test_all]
    async fn second_caller_during_cold_fetch_serializes_and_avoids_double_fetch() {
        let mock = Arc::new(MockFiat::with_delay(60_000.0, Duration::from_millis(80)));
        let cached = Arc::new(CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            DEFAULT_FIAT_CACHE_TTL,
        ));

        // Kick off the slow cold fetch.
        let first = {
            let cached = Arc::clone(&cached);
            tokio::spawn(async move { cached.fetch_fiat_rates().await.unwrap() })
        };

        // Give the first task time to acquire the lock and start awaiting the
        // upstream. Sleep is shorter than the mock's fetch delay (80ms) but
        // long enough that we know the lock is held.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Second caller fires while the first is still mid-fetch.
        let second = {
            let cached = Arc::clone(&cached);
            tokio::spawn(async move { cached.fetch_fiat_rates().await.unwrap() })
        };

        let r1 = first.await.unwrap();
        let r2 = second.await.unwrap();
        assert!((usd_rate(&r1) - 60_000.0).abs() < f64::EPSILON);
        assert!((usd_rate(&r2) - 60_000.0).abs() < f64::EPSILON);
        assert_eq!(
            mock.rates_calls(),
            1,
            "second caller fired during in-flight cold fetch must reuse the result, not double-fetch"
        );
    }

    /// On inner error, the cache stays empty so the next call retries.
    #[macros::async_test_all]
    async fn inner_error_does_not_poison_cache() {
        let mock = Arc::new(MockFiat::failing());
        let cached = CachedFiatService::new(
            Arc::clone(&mock) as Arc<dyn FiatService>,
            DEFAULT_FIAT_CACHE_TTL,
        );

        assert!(cached.fetch_fiat_rates().await.is_err());
        assert!(cached.fetch_fiat_rates().await.is_err());
        assert_eq!(
            mock.rates_calls(),
            2,
            "error path must NOT cache; next call should re-attempt the fetch"
        );
    }
}
