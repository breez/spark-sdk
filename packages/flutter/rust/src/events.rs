use crate::frb_generated::StreamSink;
pub use breez_sdk_spark::SdkEvent;
use breez_sdk_spark::{DepositInfo, EventListener, Payment};
use flutter_rust_bridge::frb;

#[frb(mirror(SdkEvent))]
pub enum _SdkEvent {
    Synced,
    ClaimDepositsFailed {
        unclaimed_deposits: Vec<DepositInfo>,
    },
    ClaimDepositsSucceeded {
        claimed_deposits: Vec<DepositInfo>,
    },
    PaymentSucceeded {
        payment: Payment,
    },
}

pub struct BindingEventListener {
    pub listener: StreamSink<SdkEvent>,
}

impl EventListener for BindingEventListener {
    fn on_event(&self, e: SdkEvent) {
        let _ = self.listener.add(e);
    }
}
