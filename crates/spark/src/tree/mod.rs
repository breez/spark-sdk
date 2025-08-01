mod error;
mod service;
mod state;

pub use error::TreeServiceError;
use serde::{Deserialize, Serialize};
pub use service::TreeService;
pub use state::TreeState;
use tracing::{error, trace};

use std::str::FromStr;

use bitcoin::{Sequence, Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;
use uuid::Uuid;

use crate::core::{TIME_LOCK_INTERVAL, next_sequence};

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
            _ => Err(format!("Unknown TreeNodeStatus: {s}")),
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
    // TODO: improve model to only allow empty refunds txs on expected cases
    pub refund_tx: Option<Transaction>,
    /// The direct transaction of the node, this transaction is for the watchtower to broadcast.
    pub direct_tx: Option<Transaction>,
    /// The refund transaction of the node, this transaction is to pay to the user.
    pub direct_refund_tx: Option<Transaction>,
    /// The refund transaction of the node, this transaction is to pay to the user.
    pub direct_from_cpfp_refund_tx: Option<Transaction>,
    /// This vout is the vout to spend the previous transaction, which is in the
    /// parent node.
    pub vout: u32,
    pub verifying_public_key: PublicKey,
    pub owner_identity_public_key: PublicKey,
    /// The signing keyshare information of the node on the SE side.
    pub signing_keyshare: SigningKeyshare,
    pub status: TreeNodeStatus,
}

impl TreeNode {
    fn is_timelock_expiring(sequence: Sequence) -> Result<bool, TreeServiceError> {
        let (next_sequence, _) = next_sequence(sequence).ok_or(TreeServiceError::Generic(
            "Failed to get next sequence".to_string(),
        ))?;
        let next_sequence_num = next_sequence.to_consensus_u32();
        Ok(next_sequence_num <= TIME_LOCK_INTERVAL as u32)
    }

    /// Checks if the node needs a timelock refresh by checking if the refund tx's timelock can be further reduced
    pub fn needs_timelock_refresh(&self) -> Result<bool, TreeServiceError> {
        let sequence = self
            .refund_tx
            .as_ref()
            .ok_or(TreeServiceError::Generic("No refund tx".to_string()))?
            .input[0]
            .sequence;
        trace!("Refund tx sequence: {sequence:?}",);
        TreeNode::is_timelock_expiring(sequence).inspect_err(|e| {
            error!("Error checking timelock refresh expiration: {:?}", e);
        })
    }

    /// Checks if the node needs a timelock extension by checking if the node tx's timelock can be further reduced
    pub fn needs_timelock_extension(&self) -> Result<bool, TreeServiceError> {
        let sequence = self.node_tx.input[0].sequence;
        trace!("Node tx sequence: {:?}", sequence);
        TreeNode::is_timelock_expiring(sequence).inspect_err(|e| {
            error!("Error checking timelock extension expiration: {:?}", e);
        })
    }
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SigningKeyshare {
    /// The identifiers of the owners of the keyshare.
    pub owner_identifiers: Vec<Identifier>,
    /// The threshold of the keyshare.
    pub threshold: u32,
    pub public_key: PublicKey,
}

type LeavesReservationId = String;

pub struct LeavesReservation {
    pub id: LeavesReservationId,
    pub leaves: Vec<TreeNode>,
}

impl LeavesReservation {
    pub fn new(leaves: Vec<TreeNode>, id: LeavesReservationId) -> Self {
        Self { leaves, id }
    }

    pub fn sum(&self) -> u64 {
        self.leaves.iter().map(|leaf| leaf.value).sum()
    }
}

#[derive(Clone, Debug)]
pub struct TargetAmounts {
    pub amount_sats: u64,
    pub fee_sats: Option<u64>,
}

impl TargetAmounts {
    pub fn new(amount_sats: u64, fee_sats: Option<u64>) -> Self {
        Self {
            amount_sats,
            fee_sats,
        }
    }

    pub fn total_sats(&self) -> u64 {
        self.amount_sats + self.fee_sats.unwrap_or(0)
    }

    pub fn to_vec(&self) -> Vec<u64> {
        let mut amounts = vec![self.amount_sats];
        if let Some(fee) = self.fee_sats {
            amounts.push(fee);
        }
        amounts
    }
}

pub struct TargetLeaves {
    pub amount_leaves: Vec<TreeNode>,
    pub fee_leaves: Option<Vec<TreeNode>>,
}

impl TargetLeaves {
    pub fn new(amount_leaves: Vec<TreeNode>, fee_leaves: Option<Vec<TreeNode>>) -> Self {
        Self {
            amount_leaves,
            fee_leaves,
        }
    }
}
