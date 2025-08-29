use std::fmt::Display;

use tokio::sync::broadcast;

use crate::{services::Transfer, tree::TreeNode};

pub type EventPublisher = broadcast::Sender<SparkEvent>;
pub type EventStream = broadcast::Receiver<SparkEvent>;

#[derive(Clone, Debug)]
pub enum SparkEvent {
    Connected,
    Disconnected,
    Transfer(Box<Transfer>),
    Deposit(Box<TreeNode>),
}

impl Display for SparkEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SparkEvent::Connected => write!(f, "Connected"),
            SparkEvent::Disconnected => write!(f, "Disconnected"),
            SparkEvent::Transfer(transfer) => write!(f, "Transfer({})", transfer.id),
            SparkEvent::Deposit(deposit) => write!(f, "Deposit({})", deposit.id),
        }
    }
}
