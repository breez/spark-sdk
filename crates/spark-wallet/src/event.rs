use spark::tree::OptimizationEvent;
use tokio::sync::broadcast;
use tracing::trace;

use crate::WalletEvent;

pub(super) struct EventManager {
    channel: broadcast::Sender<WalletEvent>,
    /// Dedicated channel for leaf-optimization lifecycle events.
    ///
    /// Kept separate from [`Self::channel`] so consumers (e.g. the SDK's
    /// optimization forwarder) can subscribe without paying the clone cost
    /// of unrelated wallet events. Only carries traffic while an
    /// optimization run is in flight.
    optimization_channel: broadcast::Sender<OptimizationEvent>,
}

impl EventManager {
    pub fn new() -> Self {
        Self {
            channel: broadcast::channel(100).0,
            optimization_channel: broadcast::channel(100).0,
        }
    }

    pub fn listen(&self) -> broadcast::Receiver<WalletEvent> {
        self.channel.subscribe()
    }

    pub fn listen_optimization(&self) -> broadcast::Receiver<OptimizationEvent> {
        self.optimization_channel.subscribe()
    }

    pub fn notify_listeners(&self, event: WalletEvent) {
        trace!("notifying listeners of event: {:?}", event);
        if self.channel.send(event).is_err() {
            tracing::debug!("Failed to send wallet event, no listeners attached");
        }
    }

    pub fn notify_optimization_listeners(&self, event: OptimizationEvent) {
        trace!("notifying optimization listeners of event: {:?}", event);
        if self.optimization_channel.send(event).is_err() {
            tracing::debug!("Failed to send optimization event, no listeners attached");
        }
    }
}
