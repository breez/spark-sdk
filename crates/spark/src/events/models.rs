use std::fmt::Display;

use tokio::sync::broadcast;

use crate::{services::Transfer, tree::TreeNode};

pub type EventPublisher = broadcast::Sender<SparkEvent>;
pub type EventStream = broadcast::Receiver<SparkEvent>;

#[derive(Clone, Debug)]
pub enum SparkEvent {
    Connected,
    Disconnected,
    ReceiverTransfer(Box<Transfer>),
    SenderTransfer(Box<Transfer>),
    Deposit(Box<TreeNode>),
    TokenTransaction { hash: String },
}

impl Display for SparkEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SparkEvent::Connected => write!(f, "Connected"),
            SparkEvent::Disconnected => write!(f, "Disconnected"),
            SparkEvent::ReceiverTransfer(transfer) => {
                write!(f, "ReceiverTransfer({})", transfer.id)
            }
            SparkEvent::SenderTransfer(transfer) => {
                write!(f, "SenderTransfer({})", transfer.id)
            }
            SparkEvent::Deposit(deposit) => write!(f, "Deposit({})", deposit.id),
            SparkEvent::TokenTransaction { hash } => write!(f, "TokenTransaction({hash})"),
        }
    }
}
