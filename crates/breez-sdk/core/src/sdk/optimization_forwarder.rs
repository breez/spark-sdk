//! Runtime-agnostic bridge from spark-wallet's dedicated
//! `OptimizationEvent` broadcast channel to `SdkEvent::Optimization` on the
//! external `EventEmitter`.
//!
//! The forwarder is gated by `BreezSdk::ensure_optimization_forwarder_spawned`,
//! which is `OnceCell`-backed so at most one task is ever spawned per SDK
//! instance. It's invoked from two places:
//!
//! - `ClientRuntime::start_sdk_services` — eagerly at startup, because client
//!   mode's `BackgroundProcessor` can trigger auto-optimization at any time.
//! - `BreezSdk::start_leaf_optimization` — lazily on first call, so server-mode
//!   SDK instances that never opt into optimization carry no forwarder task
//!   (matching the `background_tasks_enabled = false` contract).

use platform_utils::tokio;
use tokio::{select, sync::broadcast};
use tracing::{Instrument, info, warn};

use crate::{events::SdkEvent, sdk::BreezSdk};

pub(super) fn spawn_optimization_forwarder(sdk: &BreezSdk) {
    // Subscribe synchronously *before* spawning so the receiver is live by
    // the time this function returns. Otherwise a caller that invokes
    // `spark_wallet.start_leaf_optimization()` immediately after spawning
    // could race the forwarder task and lose the initial `Started` event.
    let mut optimization_events = sdk.spark_wallet.subscribe_optimization_events();
    let sdk = sdk.clone();
    let span = tracing::Span::current();

    tokio::spawn(
        async move {
            let mut shutdown = sdk.shutdown_sender.subscribe();

            loop {
                select! {
                    _ = shutdown.changed() => {
                        info!("Optimization forwarder shutdown signal received");
                        return;
                    }
                    event = optimization_events.recv() => match event {
                        Ok(e) => {
                            sdk.event_emitter
                                .emit(&SdkEvent::Optimization {
                                    optimization_event: e.into(),
                                })
                                .await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Optimization forwarder lagged by {n} events");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Optimization event stream closed; forwarder exiting");
                            return;
                        }
                    }
                }
            }
        }
        .instrument(span),
    );
}
