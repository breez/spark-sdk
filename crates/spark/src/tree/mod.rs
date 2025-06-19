use bitcoin::{Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;

use crate::Network;

pub enum TreeNodeStatus {}

pub struct TreeNode {
    pub id: String,
    pub tree_id: String,
    pub value: u64,
    pub parent_node_id: Option<String>,
    pub node_tx: Transaction,
    pub refund_tx: Transaction,
    /// This vout is the vout to spend the previous transaction, which is in the
    /// parent node.
    pub vout: usize,
    pub verifying_public_key: PublicKey,
    pub owner_identity_public_key: PublicKey,
    /// The signing keyshare information of the node on the SE side.
    pub signing_keyshare: SigningKeyshare,
    pub status: TreeNodeStatus,
    // pub network: Network,
}

pub struct SigningKeyshare {
    /// The identifiers of the owners of the keyshare.
    pub owner_identifiers: Vec<Identifier>,
    /// The threshold of the keyshare.
    pub threshold: u32,
}
