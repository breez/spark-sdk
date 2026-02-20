use std::collections::HashMap;

use crate::frb_generated::StreamSink;
use breez_sdk_spark::{
    BalanceState, DepositInfo, EventListener, Payment, SyncUpdate, TokenBalance,
};
pub use breez_sdk_spark::{OptimizationEvent, SdkEvent};
use flutter_rust_bridge::frb;

#[frb(mirror(BalanceState))]
pub struct _BalanceState {
    pub balance_sats: u64,
    pub token_balances: HashMap<String, TokenBalance>,
}

#[frb(mirror(SyncUpdate))]
pub enum _SyncUpdate {
    BalanceUpdated {
        balance: Option<BalanceState>,
    },
    PaymentsUpdated,
    FullSync,
}

#[frb(mirror(SdkEvent))]
pub enum _SdkEvent {
    Synced {
        sync_update: SyncUpdate,
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
    PaymentPending {
        payment: Payment,
    },
    PaymentFailed {
        payment: Payment,
    },
    Optimization {
        optimization_event: OptimizationEvent,
    },
}

#[frb(mirror(OptimizationEvent))]
pub enum _OptimizationEvent {
    Started {
        total_rounds: u32,
    },
    RoundCompleted {
        current_round: u32,
        total_rounds: u32,
    },
    Completed,
    Cancelled,
    Failed {
        error: String,
    },
    Skipped,
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
