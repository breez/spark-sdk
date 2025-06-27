use std::time::SystemTime;

use bitcoin::{PublicKey, Transaction};
use spark::{
    Network,
    services::{Transfer, TransferStatus, TransferType},
    tree::SigningKeyshare,
};

#[derive(Debug, Clone)]
pub struct WalletTransfer {
    pub id: String,
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
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct WalletTransferLeaf {
    pub leaf: Option<WalletLeaf>,
    pub secret_cipher: String,
    pub signature: String,
    pub intermediate_refund_tx: String,
}

#[derive(Debug, Clone)]
pub struct WalletLeaf {
    pub id: String,
    pub tree_id: String,
    pub value: u64,
    pub parent_node_id: Option<String>,
    pub node_tx: Transaction,
    pub refund_tx: Transaction,
    pub vout: u32,
    pub verifying_public_key: PublicKey,
    pub owner_identity_public_key: PublicKey,
    pub signing_keyshare: Option<SigningKeyshare>,
    pub status: String,
    pub network: Network,
}

#[derive(Debug, Clone)]
pub enum TransferDirection {
    Incoming,
    Outgoing,
}
