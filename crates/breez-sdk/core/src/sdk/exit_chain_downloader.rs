//! Schedules the collection of the data a unilateral exit needs.
//!
//! The work itself lives in the spark wallet, which knows which leaves are still
//! missing a chain and how to fetch and store one. Deciding *when* to do it is a
//! wallet-level policy, so it is driven from here: the SDK is what knows an
//! operation has finished and what the integrator configured.

use std::sync::Arc;

use platform_utils::tokio;
use platform_utils::tokio::sync::{Mutex, Notify, oneshot, watch};
use tracing::{trace, warn};

use crate::error::SdkError;

#[macros::async_trait]
impl ExitChainSource for spark_wallet::SparkWallet {
    async fn fetch_missing_exit_chains(&self) -> Result<(), SdkError> {
        Ok(spark_wallet::SparkWallet::fetch_missing_exit_chains(self).await?)
    }
}

/// The collection step this schedules. Implemented by the spark wallet; a
/// separate trait so the scheduling can be tested without one.
#[macros::async_trait]
pub(crate) trait ExitChainSource: Send + Sync {
    async fn fetch_missing_exit_chains(&self) -> Result<(), SdkError>;
}

struct Inner {
    /// Set once the downloader has stopped, so a later waiter is told rather
    /// than left waiting for a pass that will never run.
    stopped: bool,
    /// Callers waiting on the outcome of the pass that is running or about to.
    /// Drained by whichever pass finishes next, so several waiters share one.
    waiters: Vec<oneshot::Sender<Result<(), SdkError>>>,
}

/// Asks the downloader for a collection pass, and can wait for its outcome.
/// Cloneable, and cheap to hold at every call site that finishes an operation.
#[derive(Clone)]
pub(crate) struct ExitChainTrigger {
    notify: Arc<Notify>,
    inner: Arc<Mutex<Inner>>,
}

impl ExitChainTrigger {
    pub(crate) fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            inner: Arc::new(Mutex::new(Inner {
                stopped: false,
                waiters: Vec::new(),
            })),
        }
    }

    /// Asks for a collection pass and returns. Requests that arrive while one is
    /// running coalesce into a single further pass rather than queueing up.
    pub(crate) fn trigger(&self) {
        self.notify.notify_one();
    }

    /// Asks for a collection pass and waits for one to finish, reporting what it
    /// did. The pass reported on always starts after this call, so it sees
    /// whatever the caller stored before making it. Callers arriving together
    /// share one pass.
    pub(crate) async fn collect(&self) -> Result<(), SdkError> {
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = self.inner.lock().await;
            if inner.stopped {
                return Err(stopped_error());
            }
            inner.waiters.push(tx);
        }
        self.notify.notify_one();
        rx.await.unwrap_or_else(|_| Err(stopped_error()))
    }
}

fn stopped_error() -> SdkError {
    SdkError::Generic("exit chain collection is not running".to_string())
}

/// Collects until cancelled, on each request. The first pass happens without
/// waiting, so chains missed while the wallet was down are picked up at startup.
pub(crate) async fn run_downloader(
    trigger: ExitChainTrigger,
    source: Arc<dyn ExitChainSource>,
    mut cancellation_token: watch::Receiver<()>,
) {
    loop {
        // Claimed before the pass rather than after it, so a waiter is only ever
        // reported to by a pass that started once it had asked. One reported to
        // by the pass already running would learn about a snapshot taken before
        // its own leaves were stored.
        let waiters = {
            let mut inner = trigger.inner.lock().await;
            inner.waiters.drain(..).collect::<Vec<_>>()
        };
        let result = source.fetch_missing_exit_chains().await;
        if let Err(e) = &result {
            warn!("Failed to collect exit chains: {e:?}");
        }
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }

        tokio::select! {
            // Cancellation first: with a request already pending, both arms are
            // ready and shutdown should not run one more pass.
            biased;
            _ = cancellation_token.changed() => {
                trace!("Stopping exit chain downloader");
                // Dropping the senders reports the stop to anyone still waiting,
                // and the flag reports it to anyone arriving later.
                let mut inner = trigger.inner.lock().await;
                inner.stopped = true;
                inner.waiters.clear();
                return;
            }
            () = trigger.notify.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use macros::async_test_all;

    #[derive(Default)]
    struct MockSource {
        calls: AtomicUsize,
        /// When armed, the next call blocks on this until released, giving a
        /// window where a pass is genuinely in progress.
        gate: Mutex<Option<Arc<Notify>>>,
        /// When set, every call fails with this message.
        fail_with: Mutex<Option<String>>,
    }

    impl MockSource {
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        async fn arm_gate(&self) -> Arc<Notify> {
            let gate = Arc::new(Notify::new());
            *self.gate.lock().await = Some(Arc::clone(&gate));
            gate
        }

        async fn fail_with(&self, message: &str) {
            *self.fail_with.lock().await = Some(message.to_string());
        }
    }

    #[macros::async_trait]
    impl ExitChainSource for MockSource {
        async fn fetch_missing_exit_chains(&self) -> Result<(), SdkError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let gate = self.gate.lock().await.take();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            match self.fail_with.lock().await.clone() {
                Some(message) => Err(SdkError::Generic(message)),
                None => Ok(()),
            }
        }
    }

    /// Polls `condition` for up to five seconds. Counted rather than clock-based:
    /// the wasm target has no monotonic `Instant`.
    async fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..500 {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        condition()
    }

    #[async_test_all]
    async fn test_coalesced_trigger_causes_exactly_one_more_pass() {
        let source = Arc::new(MockSource::default());
        let trigger = ExitChainTrigger::new();
        let (cancel_tx, cancel_rx) = watch::channel(());

        let handle = tokio::spawn(run_downloader(
            trigger.clone(),
            Arc::clone(&source) as Arc<dyn ExitChainSource>,
            cancel_rx,
        ));

        // The loop collects once immediately, without waiting for a trigger.
        assert!(
            wait_until(|| source.call_count() >= 1).await,
            "expected the initial pass to run without a trigger"
        );
        let after_first_pass = source.call_count();
        let expected_total = after_first_pass.saturating_add(2);

        // Arm the gate and trigger once so the next pass starts and blocks
        // partway through.
        let gate = source.arm_gate().await;
        trigger.trigger();
        assert!(
            wait_until(|| source.call_count() > after_first_pass).await,
            "expected the trigger to start another pass"
        );

        // While that pass is in flight, a burst of triggers must coalesce into
        // exactly one more pass, not one per trigger.
        trigger.trigger();
        trigger.trigger();
        trigger.trigger();
        gate.notify_one();

        assert!(
            wait_until(|| source.call_count() >= expected_total).await,
            "expected the coalesced triggers to cause one further pass"
        );
        // Give a slow scheduler room to run any extra (incorrect) passes before
        // asserting the count settles instead of continuing to climb.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            source.call_count(),
            expected_total,
            "a burst of triggers must not cause a pass each"
        );

        // Cancellation has to end the loop: disconnect relies on it to avoid
        // leaking the task.
        drop(cancel_tx);
        assert!(
            wait_until(|| handle.is_finished()).await,
            "the downloader must stop when its cancellation token is dropped"
        );
    }

    /// A waiter is reported to by a pass that started after it asked, never by
    /// the one already running: that pass took its snapshot before the caller
    /// stored the leaves it is waiting to hear about.
    #[async_test_all]
    async fn test_collect_waits_for_a_pass_that_began_after_it() {
        let source = Arc::new(MockSource::default());
        let trigger = ExitChainTrigger::new();
        let (cancel_tx, cancel_rx) = watch::channel(());
        tokio::spawn(run_downloader(
            trigger.clone(),
            Arc::clone(&source) as Arc<dyn ExitChainSource>,
            cancel_rx,
        ));
        assert!(
            wait_until(|| source.call_count() >= 1).await,
            "expected the initial pass to run"
        );

        // Put a pass in flight, then ask while it is running.
        let gate = source.arm_gate().await;
        trigger.trigger();
        let before = source.call_count();
        assert!(
            wait_until(|| source.call_count() > before).await,
            "expected a pass to be in flight"
        );

        // Two callers ask while that pass is running, so both are in the list
        // when the next one claims them.
        let (a, b) = (trigger.clone(), trigger.clone());
        let first = tokio::spawn(async move { a.collect().await });
        let second = tokio::spawn(async move { b.collect().await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let in_flight = source.call_count();

        // Hold the follow-up pass too, so releasing the in-flight one is the
        // only thing that could report to them. If it does, that is the
        // regression: they heard about a snapshot older than their request.
        let next_gate = source.arm_gate().await;
        gate.notify_one();
        assert!(
            wait_until(|| source.call_count() > in_flight).await,
            "expected a further pass to start for the waiters"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !first.is_finished() && !second.is_finished(),
            "collect must not be reported to by a pass that began before it"
        );
        let serving = source.call_count();

        next_gate.notify_one();
        assert!(
            first.await.expect("waiter panicked").is_ok(),
            "a successful pass must report success"
        );
        assert!(
            second.await.expect("waiter panicked").is_ok(),
            "both waiters must be reported to by the same pass"
        );
        assert_eq!(
            source.call_count(),
            serving,
            "callers waiting together must share one pass"
        );

        // A failing pass reports its error rather than a bare success.
        source.fail_with("operators unreachable").await;
        let failed = trigger.collect().await;
        assert!(
            matches!(failed, Err(SdkError::Generic(m)) if m == "operators unreachable"),
            "a failed pass must report its error"
        );

        // A waiter must not hang once the downloader is gone.
        drop(cancel_tx);
        let orphaned = trigger.clone();
        let after_shutdown = tokio::spawn(async move { orphaned.collect().await });
        assert!(
            wait_until(|| after_shutdown.is_finished()).await,
            "collect must return once the downloader has stopped"
        );
    }
}
