use std::time::SystemTime;

use bitcoin::{Transaction, secp256k1::PublicKey};
use serde::{Deserialize, Serialize};
use spark::{
    Network,
    services::{Transfer, TransferId, TransferLeaf, TransferStatus, TransferType},
    tree::{SigningKeyshare, TreeNode, TreeNodeId},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletInfo {
    pub identity_public_key: PublicKey,
    pub network: Network,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletTransfer {
    pub id: TransferId,
    pub sender_id: PublicKey,
    pub receiver_id: PublicKey,
    pub status: TransferStatus,
    pub total_value_sat: u64,
    pub expiry_time: Option<SystemTime>,
    pub leaves: Vec<WalletTransferLeaf>,
    pub created_at: Option<SystemTime>,
    pub updated_at: Option<SystemTime>,
    pub transfer_type: TransferType,
    pub direction: TransferDirection,
}

impl From<Transfer> for WalletTransfer {
    fn from(value: Transfer) -> Self {
        WalletTransfer {
            id: value.id,
            sender_id: value.sender_identity_public_key,
            receiver_id: value.receiver_identity_public_key,
            status: value.status,
            total_value_sat: value.total_value,
            expiry_time: None,
            leaves: value.leaves.into_iter().map(Into::into).collect(),
            created_at: None,
            updated_at: None,
            transfer_type: value.transfer_type,
            direction: TransferDirection::default(), // TODO: Set to actual direction
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletTransferLeaf {
    pub leaf: WalletLeaf,
    // pub secret_cipher: String,
    // pub signature: String,
    // pub intermediate_refund_tx: String,
}

impl From<TransferLeaf> for WalletTransferLeaf {
    fn from(value: TransferLeaf) -> Self {
        WalletTransferLeaf {
            leaf: value.leaf.into(),
            // secret_cipher: value.secret_cipher,
            // signature: value.signature,
            // intermediate_refund_tx: value.intermediate_refund_tx,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletLeaf {
    pub id: TreeNodeId,
    pub tree_id: String,
    pub value: u64,
    pub parent_node_id: Option<TreeNodeId>,
    pub node_tx: Transaction,
    pub refund_tx: Option<Transaction>,
    pub vout: u32,
    pub verifying_public_key: PublicKey,
    pub owner_identity_public_key: PublicKey,
    pub signing_keyshare: Option<SigningKeyshare>,
    pub status: String,
}

impl From<TreeNode> for WalletLeaf {
    fn from(value: TreeNode) -> Self {
        WalletLeaf {
            id: value.id,
            tree_id: value.tree_id,
            value: value.value,
            parent_node_id: value.parent_node_id,
            node_tx: value.node_tx,
            refund_tx: value.refund_tx,
            vout: value.vout,
            verifying_public_key: value.verifying_public_key,
            owner_identity_public_key: value.owner_identity_public_key,
            signing_keyshare: Some(value.signing_keyshare),
            status: format!("{:?}", value.status),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub enum TransferDirection {
    #[default]
    Unknown,
    Incoming,
    Outgoing,
}
