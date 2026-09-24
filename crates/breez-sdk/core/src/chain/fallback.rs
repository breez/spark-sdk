use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use platform_utils::time::Instant;
use tracing::warn;

use super::{BitcoinChainService, ChainServiceError, Outspend, RecommendedFees, TxStatus, Utxo};

/// How long a backend that failed while a later one succeeded is first tried
/// last. Doubles each time it is demoted again, up to [`MAX_DEMOTION_PERIOD`].
#[allow(clippy::duration_suboptimal_units)]
const BASE_DEMOTION_PERIOD: Duration = Duration::from_secs(300);
#[allow(clippy::duration_suboptimal_units)]
const MAX_DEMOTION_PERIOD: Duration = Duration::from_secs(3600);

/// Serves each call from the first backend, in priority order, that answers it.
///
/// A backend that fails where a later one succeeds is demoted to the end of
/// the order for a while, so an unreachable backend stops costing a timeout on
/// every call. A backend that answers is no longer demoted. When every backend fails the request is assumed
/// to be at fault (e.g. an invalid transaction broadcast) and nothing is demoted.
pub(crate) struct FallbackChainService {
    backends: Vec<Arc<dyn BitcoinChainService>>,
    demotions: Mutex<Vec<Demotion>>,
}

#[derive(Clone, Copy, Default)]
struct Demotion {
    until: Option<Instant>,
    /// Consecutive demotions without an answer in between.
    count: u32,
}

impl FallbackChainService {
    pub(crate) fn new(backends: Vec<Arc<dyn BitcoinChainService>>) -> Self {
        let demotions = Mutex::new(vec![Demotion::default(); backends.len()]);
        Self {
            backends,
            demotions,
        }
    }

    /// Backend indexes in priority order, demoted ones last.
    fn attempt_order(&self) -> Vec<usize> {
        let now = Instant::now();
        let demotions = self
            .demotions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (healthy, demoted): (Vec<usize>, Vec<usize>) = (0..self.backends.len())
            .partition(|&i| demotions[i].until.is_none_or(|until| until <= now));
        healthy.into_iter().chain(demoted).collect()
    }

    fn record_success(&self, succeeded: usize, failed: &[usize]) {
        let mut demotions = self
            .demotions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        demotions[succeeded] = Demotion::default();
        let now = Instant::now();
        for &i in failed {
            let demotion = &mut demotions[i];
            demotion.until = now.checked_add(demotion_period(demotion.count));
            demotion.count = demotion.count.saturating_add(1);
        }
    }

    async fn call<T, F, Fut>(&self, op: F) -> Result<T, ChainServiceError>
    where
        F: Fn(Arc<dyn BitcoinChainService>) -> Fut,
        Fut: Future<Output = Result<T, ChainServiceError>>,
    {
        let mut failed = Vec::new();
        let mut first_error = None;
        for i in self.attempt_order() {
            match op(self.backends[i].clone()).await {
                Ok(value) => {
                    self.record_success(i, &failed);
                    return Ok(value);
                }
                Err(e) if should_fall_back(&e) => {
                    warn!("Chain service backend {i} failed, trying the next one: {e}");
                    failed.push(i);
                    // Later backends are the demoted ones, so the first error
                    // comes from the most trusted backend.
                    first_error.get_or_insert(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(first_error
            .unwrap_or_else(|| ChainServiceError::Generic("no chain service backends".to_string())))
    }
}

/// [`BASE_DEMOTION_PERIOD`] doubled once per earlier consecutive demotion.
fn demotion_period(earlier_demotions: u32) -> Duration {
    BASE_DEMOTION_PERIOD
        .checked_mul(2u32.saturating_pow(earlier_demotions))
        .map_or(MAX_DEMOTION_PERIOD, |period| {
            period.min(MAX_DEMOTION_PERIOD)
        })
}

/// `NotFound` and `InvalidAddress` are answers about the request, which
/// another backend would repeat. Anything else may be specific to the backend.
fn should_fall_back(error: &ChainServiceError) -> bool {
    matches!(
        error,
        ChainServiceError::ServiceConnectivity(_) | ChainServiceError::Generic(_)
    )
}

#[macros::async_trait]
impl BitcoinChainService for FallbackChainService {
    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        self.call(|b| {
            let address = address.clone();
            async move { b.get_address_utxos(address).await }
        })
        .await
    }

    async fn get_address_txos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        self.call(|b| {
            let address = address.clone();
            async move { b.get_address_txos(address).await }
        })
        .await
    }

    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError> {
        self.call(|b| {
            let txid = txid.clone();
            async move { b.get_transaction_status(txid).await }
        })
        .await
    }

    async fn tip_height(&self) -> Result<u32, ChainServiceError> {
        self.call(|b| async move { b.tip_height().await }).await
    }

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        self.call(|b| {
            let txid = txid.clone();
            async move { b.get_transaction_hex(txid).await }
        })
        .await
    }

    async fn get_outspend(&self, txid: String, vout: u32) -> Result<Outspend, ChainServiceError> {
        self.call(|b| {
            let txid = txid.clone();
            async move { b.get_outspend(txid, vout).await }
        })
        .await
    }

    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError> {
        self.call(|b| {
            let tx = tx.clone();
            async move { b.broadcast_transaction(tx).await }
        })
        .await
    }

    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
        self.call(|b| async move { b.recommended_fees().await })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// Answers `tip_height` with a fixed result and counts the calls.
    /// Every other method is unreachable in these tests.
    struct TipChainService {
        result: Result<u32, ChainServiceError>,
        calls: AtomicUsize,
    }

    impl TipChainService {
        fn new(result: Result<u32, ChainServiceError>) -> Arc<Self> {
            Arc::new(Self {
                result,
                calls: AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[macros::async_trait]
    impl BitcoinChainService for TipChainService {
        async fn get_address_utxos(
            &self,
            _address: String,
        ) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!()
        }

        async fn get_address_txos(&self, _address: String) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!()
        }

        async fn get_transaction_status(
            &self,
            _txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            unreachable!()
        }

        async fn tip_height(&self) -> Result<u32, ChainServiceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }

        async fn get_transaction_hex(&self, _txid: String) -> Result<String, ChainServiceError> {
            unreachable!()
        }

        async fn get_outspend(
            &self,
            _txid: String,
            _vout: u32,
        ) -> Result<Outspend, ChainServiceError> {
            unreachable!()
        }

        async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
            unreachable!()
        }

        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            unreachable!()
        }
    }

    fn down() -> Result<u32, ChainServiceError> {
        Err(ChainServiceError::ServiceConnectivity("down".to_string()))
    }

    fn fallback(backends: &[&Arc<TipChainService>]) -> FallbackChainService {
        FallbackChainService::new(
            backends
                .iter()
                .map(|b| (*b).clone() as Arc<dyn BitcoinChainService>)
                .collect(),
        )
    }

    #[async_test_all]
    async fn uses_primary_while_it_answers() {
        let primary = TipChainService::new(Ok(1));
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);

        assert_eq!(service.tip_height().await.unwrap(), 1);
        assert_eq!(secondary.calls(), 0);
    }

    #[async_test_all]
    async fn falls_back_and_demotes_unreachable_primary() {
        let primary = TipChainService::new(down());
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);

        assert_eq!(service.tip_height().await.unwrap(), 2);
        assert_eq!(service.tip_height().await.unwrap(), 2);
        // Demoted after the first failure, so the second call skips it.
        assert_eq!(primary.calls(), 1);
        assert_eq!(secondary.calls(), 2);
    }

    #[async_test_all]
    async fn falls_back_on_generic_errors() {
        let primary = TipChainService::new(Err(ChainServiceError::Generic("garbage".into())));
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);

        assert_eq!(service.tip_height().await.unwrap(), 2);
    }

    #[async_test_all]
    async fn does_not_fall_back_on_not_found() {
        let primary = TipChainService::new(Err(ChainServiceError::NotFound("nope".into())));
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);

        assert!(matches!(
            service.tip_height().await,
            Err(ChainServiceError::NotFound(_))
        ));
        assert_eq!(secondary.calls(), 0);
    }

    #[async_test_all]
    async fn all_failing_returns_first_error_and_demotes_nothing() {
        let primary = TipChainService::new(Err(ChainServiceError::Generic("first".into())));
        let secondary = TipChainService::new(down());
        let service = fallback(&[&primary, &secondary]);

        assert!(matches!(
            service.tip_height().await,
            Err(ChainServiceError::Generic(message)) if message == "first"
        ));
        assert_eq!(service.attempt_order(), vec![0, 1]);
    }

    #[async_test_all]
    async fn demoted_backend_is_still_tried_last() {
        let primary = TipChainService::new(down());
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);
        service.record_success(1, &[0]);

        assert_eq!(service.attempt_order(), vec![1, 0]);
    }

    #[async_test_all]
    async fn demotion_expires() {
        let primary = TipChainService::new(Ok(1));
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);
        service.demotions.lock().unwrap()[0].until = Some(Instant::now());

        assert_eq!(service.attempt_order(), vec![0, 1]);
    }

    #[test]
    fn demotion_period_doubles_up_to_the_cap() {
        assert_eq!(demotion_period(0), BASE_DEMOTION_PERIOD);
        assert_eq!(demotion_period(1), BASE_DEMOTION_PERIOD * 2);
        assert_eq!(demotion_period(2), BASE_DEMOTION_PERIOD * 4);
        assert_eq!(demotion_period(4), MAX_DEMOTION_PERIOD);
        assert_eq!(demotion_period(u32::MAX), MAX_DEMOTION_PERIOD);
    }

    #[async_test_all]
    async fn repeated_demotions_count_until_an_answer() {
        let primary = TipChainService::new(Ok(1));
        let secondary = TipChainService::new(Ok(2));
        let service = fallback(&[&primary, &secondary]);
        service.record_success(1, &[0]);
        service.record_success(1, &[0]);
        assert_eq!(service.demotions.lock().unwrap()[0].count, 2);

        service.record_success(0, &[]);
        let demotion = service.demotions.lock().unwrap()[0];
        assert_eq!(demotion.count, 0);
        assert!(demotion.until.is_none());
    }
}
