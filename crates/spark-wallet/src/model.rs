use std::{
    fmt::Display,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitcoin::{Transaction, secp256k1::PublicKey};
use serde::{Deserialize, Serialize};
use spark::{
    Network,
    services::{
        LightningSendPayment, TokenMetadata, Transfer, TransferId, TransferLeaf, TransferStatus,
        TransferType,
    },
    ssp::{SspTransfer, SspUserRequest},
    tree::{SigningKeyshare, TreeNode, TreeNodeId},
    utils::paging::PagingFilter,
};

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum WalletEvent {
    DepositConfirmed(TreeNodeId),
    StreamConnected,
    StreamDisconnected,
    Synced,
    TransferClaimed(WalletTransfer),
}

impl Display for WalletEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let WalletEvent::TransferClaimed(transfer) = self {
            write!(f, "TransferClaimed({})", transfer.id)
        } else {
            write!(f, "{:?}", self)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletInfo {
    pub identity_public_key: PublicKey,
    pub network: Network,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
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
    pub user_request: Option<SspUserRequest>,
}

impl WalletTransfer {
    pub fn from_transfer(
        value: Transfer,
        ssp_transfer: Option<SspTransfer>,
        our_public_key: PublicKey,
    ) -> Self {
        let direction = if value.sender_identity_public_key == our_public_key {
            TransferDirection::Outgoing
        } else {
            TransferDirection::Incoming
        };
        WalletTransfer {
            id: value.id,
            sender_id: value.sender_identity_public_key,
            receiver_id: value.receiver_identity_public_key,
            status: value.status,
            total_value_sat: value.total_value,
            expiry_time: value
                .expiry_time
                .map(|t| UNIX_EPOCH + Duration::from_secs(t)),
            leaves: value.leaves.into_iter().map(Into::into).collect(),
            created_at: value
                .created_time
                .map(|t| UNIX_EPOCH + Duration::from_secs(t)),
            updated_at: value
                .updated_time
                .map(|t| UNIX_EPOCH + Duration::from_secs(t)),
            transfer_type: value.transfer_type,
            direction,
            user_request: ssp_transfer.and_then(|t| t.user_request),
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize)]
pub enum PayLightningInvoiceResult {
    LightningPayment(LightningSendPayment),
    Transfer(WalletTransfer),
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct WalletTransferLeaf {
    pub leaf: WalletLeaf,
    pub secret_cipher: String,
    pub signature: String,
    pub intermediate_refund_tx: String,
}

impl From<TransferLeaf> for WalletTransferLeaf {
    fn from(value: TransferLeaf) -> Self {
        WalletTransferLeaf {
            leaf: value.leaf.into(),
            secret_cipher: hex::encode(value.secret_cipher),
            signature: value
                .signature
                .map(|s| hex::encode(s.serialize_compact()))
                .unwrap_or_default(),
            intermediate_refund_tx: hex::encode(bitcoin::consensus::serialize(
                &value.intermediate_refund_tx,
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct WalletLeaf {
    pub id: TreeNodeId,
    pub tree_id: String,
    pub value: u64,
    pub parent_node_id: Option<TreeNodeId>,
    pub node_tx: Transaction,
    pub refund_tx: Option<Transaction>,
    pub direct_tx: Option<Transaction>,
    pub direct_refund_tx: Option<Transaction>,
    pub direct_from_cpfp_refund_tx: Option<Transaction>,
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
            direct_tx: value.direct_tx,
            direct_refund_tx: value.direct_refund_tx,
            direct_from_cpfp_refund_tx: value.direct_from_cpfp_refund_tx,
            vout: value.vout,
            verifying_public_key: value.verifying_public_key,
            owner_identity_public_key: value.owner_identity_public_key,
            signing_keyshare: Some(value.signing_keyshare),
            status: format!("{:?}", value.status),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum TransferDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenBalance {
    pub balance: u128,
    pub token_metadata: TokenMetadata,
}

#[derive(Default)]
pub struct ListTokenTransactionsRequest {
    pub paging: Option<PagingFilter>,
    /// If not provided, will use our own public key
    pub owner_public_keys: Option<Vec<PublicKey>>,
    pub issuer_public_keys: Vec<PublicKey>,
    pub token_transaction_hashes: Vec<String>,
    pub token_ids: Vec<String>,
    pub output_ids: Vec<String>,
}
