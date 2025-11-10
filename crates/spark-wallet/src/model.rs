use std::{
    fmt::Display,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitcoin::{Transaction, secp256k1::PublicKey};
use serde::{Deserialize, Serialize};
use spark::{
    Network,
    operator::rpc::spark::{
        InvoiceResponse, InvoiceStatus, WalletSetting,
        invoice_response::TransferType as InvoiceTransferType,
    },
    services::{
        LightningSendPayment, TokenMetadata, TokenTransaction, Transfer, TransferId, TransferLeaf,
        TransferStatus, TransferType,
    },
    ssp::{SspTransfer, SspUserRequest},
    tree::{Leaves, SigningKeyshare, TreeNode, TreeNodeId},
    utils::paging::PagingFilter,
};

use crate::SparkWalletError;

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum WalletEvent {
    DepositConfirmed(TreeNodeId),
    StreamConnected,
    StreamDisconnected,
    Synced,
    TransferClaimed(WalletTransfer),
    TransferClaimStarting(WalletTransfer),
}

impl Display for WalletEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let WalletEvent::TransferClaimed(transfer) = self {
            write!(f, "TransferClaimed({})", transfer.id)
        } else if let WalletEvent::TransferClaimStarting(transfer) = self {
            write!(f, "TransferClaimStarting({})", transfer.id)
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
    pub spark_invoice: Option<String>,
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
            spark_invoice: value.spark_invoice,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PayLightningInvoiceResult {
    // The transfer associated with this lightinng payment.
    pub transfer: WalletTransfer,
    // The optional ssp lightning payment if the payment was created with the ssp.
    pub lightning_payment: Option<LightningSendPayment>,
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
pub struct WalletLeaves {
    pub available: Vec<WalletLeaf>,
    pub available_missing_from_operators: Vec<WalletLeaf>,
}

impl From<Leaves> for WalletLeaves {
    fn from(value: Leaves) -> Self {
        WalletLeaves {
            available: value.available.into_iter().map(Into::into).collect(),
            available_missing_from_operators: value
                .available_missing_from_operators
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl WalletLeaves {
    pub fn available_balance(&self) -> u64 {
        self.available.iter().map(|leaf| leaf.value).sum()
    }
    pub fn missing_operators_balance(&self) -> u64 {
        self.available_missing_from_operators
            .iter()
            .map(|leaf| leaf.value)
            .sum()
    }
    pub fn balance(&self) -> u64 {
        self.available_balance() + self.missing_operators_balance()
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum FulfillSparkInvoiceResult {
    Transfer(Box<WalletTransfer>),
    TokenTransaction(Box<TokenTransaction>),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QuerySparkInvoiceResult {
    pub invoice: String,
    pub status: SparkInvoiceStatus,
    pub transfer_type: Option<SparkInvoiceTransferType>,
}

impl TryFrom<InvoiceResponse> for QuerySparkInvoiceResult {
    type Error = SparkWalletError;
    fn try_from(value: InvoiceResponse) -> Result<Self, Self::Error> {
        Ok(QuerySparkInvoiceResult {
            invoice: value.invoice,
            status: InvoiceStatus::try_from(value.status)
                .map_err(|_| {
                    SparkWalletError::ValidationError("Invalid invoice status".to_string())
                })?
                .into(),
            transfer_type: value.transfer_type.map(TryInto::try_into).transpose()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SparkInvoiceStatus {
    NotFound,
    Pending,
    Finalized,
    Returned,
}

impl From<InvoiceStatus> for SparkInvoiceStatus {
    fn from(value: InvoiceStatus) -> Self {
        match value {
            InvoiceStatus::NotFound => SparkInvoiceStatus::NotFound,
            InvoiceStatus::Pending => SparkInvoiceStatus::Pending,
            InvoiceStatus::Finalized => SparkInvoiceStatus::Finalized,
            InvoiceStatus::Returned => SparkInvoiceStatus::Returned,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SparkInvoiceTransferType {
    Transfer { transfer_id: TransferId },
    TokenTransfer { final_token_tx_hash: String },
}

impl TryFrom<InvoiceTransferType> for SparkInvoiceTransferType {
    type Error = SparkWalletError;
    fn try_from(value: InvoiceTransferType) -> Result<Self, Self::Error> {
        match value {
            InvoiceTransferType::SatsTransfer(transfer) => Ok(SparkInvoiceTransferType::Transfer {
                transfer_id: TransferId::from_bytes(transfer.transfer_id.try_into().map_err(
                    |e| {
                        SparkWalletError::ValidationError(format!(
                            "Failed to convert id bytes to UUID: {e:?}"
                        ))
                    },
                )?),
            }),

            InvoiceTransferType::TokenTransfer(transfer) => {
                Ok(SparkInvoiceTransferType::TokenTransfer {
                    final_token_tx_hash: hex::encode(transfer.final_token_transaction_hash),
                })
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WalletSettings {
    pub private_enabled: bool,
}

impl From<WalletSetting> for WalletSettings {
    fn from(value: WalletSetting) -> Self {
        WalletSettings {
            private_enabled: value.private_enabled,
        }
    }
}

pub struct IssuerTokenBalance {
    pub identifier: String,
    pub balance: u128,
}
