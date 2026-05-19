use std::{future::Future, time::Duration};

use platform_utils::{
    time::Instant,
    tokio::{self, sync::watch, time::sleep},
};

use crate::error::SdkError;

pub(crate) struct PollSchedule {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub timeout: Duration,
}

/// Probes `f` until it returns `Ok(Some(T))` or the timeout elapses.
///
/// Errors from `f` are treated like `Ok(None)` — the helper keeps probing
/// and surfaces the last error on timeout, or a generic "Timeout while
/// polling" if every probe returned `Ok(None)`. Between probes, sleeps
/// `initial_delay`, doubling each iteration up to `max_delay`. If
/// `shutdown` is provided and fires, aborts immediately.
pub(crate) async fn poll_until<T, F, Fut>(
    schedule: PollSchedule,
    mut shutdown: Option<watch::Receiver<()>>,
    mut f: F,
) -> Result<T, SdkError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>, SdkError>>,
{
    let started = Instant::now();
    let mut delay = schedule.initial_delay;
    let mut last_error: Option<SdkError>;

    loop {
        let probe = f();
        // `biased;` polls the shutdown branch first on each iteration, so a
        // shutdown signal racing with a ready probe always wins. Without it,
        // tokio's randomised select could let the probe finish a final
        // round-trip after we've been asked to stop.
        let outcome = match shutdown.as_mut() {
            Some(rx) => tokio::select! {
                biased;
                _ = rx.changed() => {
                    return Err(SdkError::Generic(
                        "Shutdown received while polling".to_string(),
                    ));
                }
                r = probe => r,
            },
            None => probe.await,
        };

        match outcome {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => last_error = None,
            Err(e) => last_error = Some(e),
        }

        let remaining = schedule.timeout.saturating_sub(started.elapsed());
        let sleep_for = delay.min(remaining);
        match shutdown.as_mut() {
            Some(rx) => tokio::select! {
                biased;
                _ = rx.changed() => {
                    return Err(SdkError::Generic(
                        "Shutdown received while polling".to_string(),
                    ));
                }
                () = sleep(sleep_for) => {},
            },
            None => sleep(sleep_for).await,
        }

        if started.elapsed() >= schedule.timeout {
            return Err(last_error
                .unwrap_or_else(|| SdkError::Generic("Timeout while polling".to_string())));
        }

        delay = delay.saturating_mul(2).min(schedule.max_delay);
    }
}
