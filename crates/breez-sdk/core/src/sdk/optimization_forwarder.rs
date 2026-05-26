//! Runtime-agnostic bridge from spark-wallet's dedicated
//! `OptimizationEvent` broadcast channel to `SdkEvent::Optimization` on the
//! external `EventEmitter`.
//!
//! Spawned from `BreezSdk::start` so it runs in both client and server mode —
//! the public `start_leaf_optimization` API is available in both, and listeners
//! should see progress events in both. The dedicated channel is silent when
//! no optimization is running, so the forwarder has no per-event cost in the
//! common case.

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
