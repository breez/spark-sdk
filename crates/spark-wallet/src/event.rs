use tokio::sync::broadcast;
use tracing::trace;

use crate::WalletEvent;

pub(super) struct EventManager {
    channel: broadcast::Sender<WalletEvent>,
}

impl EventManager {
    pub fn new() -> Self {
        Self {
            channel: broadcast::channel(100).0,
        }
    }

    pub fn listen(&self) -> broadcast::Receiver<WalletEvent> {
        self.channel.subscribe()
    }

    pub fn notify_listeners(&self, event: WalletEvent) {
        trace!("notifying listeners of event: {:?}", event);
        if self.channel.send(event).is_err() {
            tracing::debug!("Failed to send wallet event, no listeners attached");
        }
    }
}
