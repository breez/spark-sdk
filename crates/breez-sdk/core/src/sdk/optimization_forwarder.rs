//! Runtime-agnostic bridge from `WalletEvent::Optimization` (emitted by the
//! spark-wallet leaf optimizer) to `SdkEvent::Optimization` on the external
//! `EventEmitter`.
//!
//! Spawned from `BreezSdk::start` so it runs in both client and server mode —
//! the public `start_leaf_optimization` API is available in both, and listeners
//! should see progress events in both.

use platform_utils::tokio;
use spark_wallet::WalletEvent;
use tokio::{select, sync::broadcast};
use tracing::{Instrument, info, warn};

use crate::{
    events::SdkEvent,
    sdk::BreezSdk,
};

pub(super) fn spawn_optimization_forwarder(sdk: &BreezSdk) {
    let sdk = sdk.clone();
    let span = tracing::Span::current();

    tokio::spawn(
        async move {
            let mut wallet_events = sdk.spark_wallet.subscribe_events();
            let mut shutdown = sdk.shutdown_sender.subscribe();

            loop {
                select! {
                    _ = shutdown.changed() => {
                        info!("Optimization forwarder shutdown signal received");
                        return;
                    }
                    event = wallet_events.recv() => match event {
                        Ok(WalletEvent::Optimization(e)) => {
                            sdk.event_emitter
                                .emit(&SdkEvent::Optimization {
                                    optimization_event: e.into(),
                                })
                                .await;
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Optimization forwarder lagged by {n} wallet events");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Wallet event stream closed; optimization forwarder exiting");
                            return;
                        }
                    }
                }
            }
        }
        .instrument(span),
    );
}
