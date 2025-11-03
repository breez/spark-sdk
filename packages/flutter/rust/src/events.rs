use crate::frb_generated::StreamSink;
pub use breez_sdk_spark::SdkEvent;
use breez_sdk_spark::{DepositInfo, EventListener, Payment};
use flutter_rust_bridge::frb;

#[frb(mirror(SdkEvent))]
pub enum _SdkEvent {
    Synced,
    DataSynced {
        did_pull_new_records: bool,
    },
    UnclaimedDeposits {
        unclaimed_deposits: Vec<DepositInfo>,
    },
    ClaimedDeposits {
        claimed_deposits: Vec<DepositInfo>,
    },
    PaymentSucceeded {
        payment: Payment,
    },
    PaymentFailed {
        payment: Payment,
    },
}

pub struct BindingEventListener {
    pub listener: StreamSink<SdkEvent>,
}

#[async_trait::async_trait]
impl EventListener for BindingEventListener {
    async fn on_event(&self, e: SdkEvent) {
        let _ = self.listener.add(e);
    }
}
