use std::future::Future;

use tracing::Instrument;

use crate::tokio::task::JoinHandle;

/// Spawns `future` as a task that runs in the span current at the call, so
/// what it logs is attributed like the code that spawned it.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    #[allow(clippy::disallowed_methods)]
    tokio::spawn(future.in_current_span())
}

/// Spawns `future` as a task that runs in the span current at the call, so
/// what it logs is attributed like the code that spawned it.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    crate::tokio::spawn(future.in_current_span())
}
