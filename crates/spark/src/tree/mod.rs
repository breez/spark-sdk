mod error;
mod service;
mod state;

pub use error::TreeServiceError;
use serde::{Deserialize, Serialize};
pub use service::TreeService;
pub use state::TreeState;

use std::str::FromStr;

use bitcoin::{Sequence, Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum TreeNodeStatus {
    /// Creating is the status of a tree node that is under creation.
    Creating,
    /// Available is the status of a tree node that is available.
    Available,
    /// FrozenByIssuer is the status of a tree node that is frozen by the issuer.
    FrozenByIssuer,
    /// TransferLocked is the status of a tree node that is transfer locked.
    TransferLocked,
    /// SplitLocked is the status of a tree node that is split locked.
    SplitLocked,
    /// Splitted is the status of a tree node that is splitted.
    Splitted,
    /// Aggregated is the status of a tree node that is aggregated, this is a terminal state.
    Aggregated,
    /// OnChain is the status of a tree node that is on chain, this is a terminal state.
    OnChain,
    /// Exited is the status of a tree node where the whole tree exited, this is a terminal state.
    Exited,
    /// AggregateLock is the status of a tree node that is aggregate locked.
    AggregateLock,
    /// Investigation is the status of a tree node that is investigated.
    Investigation,
    /// Lost is the status of a tree node that is in a unrecoverable bad state.
    Lost,
    /// Reimbursed is the status of a tree node that is reimbursed after LOST.
    Reimbursed,
}

impl std::str::FromStr for TreeNodeStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "CREATING" => Ok(TreeNodeStatus::Creating),
            "AVAILABLE" => Ok(TreeNodeStatus::Available),
            "FROZEN_BY_ISSUER" => Ok(TreeNodeStatus::FrozenByIssuer),
            "TRANSFER_LOCKED" => Ok(TreeNodeStatus::TransferLocked),
            "SPLIT_LOCKED" => Ok(TreeNodeStatus::SplitLocked),
            "SPLITTED" => Ok(TreeNodeStatus::Splitted),
            "AGGREGATED" => Ok(TreeNodeStatus::Aggregated),
            "ON_CHAIN" => Ok(TreeNodeStatus::OnChain),
            "EXITED" => Ok(TreeNodeStatus::Exited),
            "AGGREGATE_LOCK" => Ok(TreeNodeStatus::AggregateLock),
            "INVESTIGATION" => Ok(TreeNodeStatus::Investigation),
            "LOST" => Ok(TreeNodeStatus::Lost),
            "REIMBURSED" => Ok(TreeNodeStatus::Reimbursed),
            _ => Err(format!("Unknown TreeNodeStatus: {}", s)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub id: TreeNodeId,
    pub tree_id: String,
    pub value: u64,
    pub parent_node_id: Option<TreeNodeId>,
    pub node_tx: Transaction,
    pub refund_tx: Transaction,
    /// This vout is the vout to spend the previous transaction, which is in the
    /// parent node.
    pub vout: u32,
    pub verifying_public_key: PublicKey,
    pub owner_identity_public_key: PublicKey,
    /// The signing keyshare information of the node on the SE side.
    pub signing_keyshare: SigningKeyshare,
    pub status: TreeNodeStatus,
    // pub network: Network,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TreeNodeId(String);

impl TreeNodeId {
    pub fn generate() -> Self {
        Self(Uuid::now_v7().to_string())
    }
}

impl std::fmt::Display for TreeNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TreeNodeId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("TreeNodeId cannot be empty".to_string());
        }
        Ok(TreeNodeId(s.to_string()))
    }
}

pub struct TreeNodeTransactionSequence {
    pub next_sequence: Sequence,
    pub needs_refresh: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SigningKeyshare {
    /// The identifiers of the owners of the keyshare.
    pub owner_identifiers: Vec<Identifier>,
    /// The threshold of the keyshare.
    pub threshold: u32,
}
