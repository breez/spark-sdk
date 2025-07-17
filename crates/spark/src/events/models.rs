use tokio::sync::broadcast;

use crate::{services::Transfer, tree::TreeNode};

pub type EventPublisher = broadcast::Sender<SparkEvent>;
pub type EventStream = broadcast::Receiver<SparkEvent>;

#[derive(Clone, Debug)]
pub enum SparkEvent {
    Transfer(Transfer),
    Deposit(TreeNode),
}
