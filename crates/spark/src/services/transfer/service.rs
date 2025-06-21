use std::collections::HashMap;

use bitcoin::{Transaction, secp256k1::PublicKey};

use crate::{services::transfer::TransferServiceError, tree::TreeNode};

pub struct LeafKeyTweak {
    pub node: TreeNode,
    pub signing_public_key: PublicKey,
    pub new_signing_public_key: PublicKey,
}

pub struct Transfer {
    pub id: String,
    pub sender_identity_public_key: PublicKey,
    pub receiver_identity_public_key: PublicKey,
    pub status: TransferStatus,
    pub total_value: u64,
    pub expiry_time: u64,
    pub leaves: Vec<TransferLeaf>,
    pub created_time: u64,
    pub updated_time: u64,
    pub transfer_type: TransferType,
}

pub struct TransferLeaf {
    pub leaf: TreeNode,
    pub secret_cipher: Vec<u8>,
    pub signature: Vec<u8>,
    pub intermediate_refund_tx: Transaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferStatus {
    Unrecognized,
    SenderInitiated,
    SenderKeyTweakPending,
    SenderKeyTweaked,
    ReceiverKeyTweaked,
    ReceiverRefundSigned,
    Completed,
    Expired,
    Returned,
    SenderInitiatedCoordinator,
    ReceiverKeyTweakLocked,
    ReceiverKeyTweakApplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferType {
    Unrecognized,
    PreimageSwap,
    CooperativeExit,
    Transfer,
    UtxoSwap,
    Swap,
    CounterSwap,
}

pub struct TransferService<S> {
    signer: S,
}

impl<S> TransferService<S> {
    pub fn new(signer: S) -> Self {
        Self { signer }
    }

    pub async fn claim_transfer(
        &self,
        transfer: &Transfer,
        leaves_to_claim: Vec<LeafKeyTweak>,
    ) -> Result<Vec<TreeNode>, TransferServiceError> {
        todo!()
    }

    pub async fn extend_time_lock(
        &self,
        node: &TreeNode,
        signing_public_key: &PublicKey,
    ) -> Result<Vec<TreeNode>, TransferServiceError> {
        todo!()
    }

    pub async fn send_transfer_with_key_tweaks(
        &self,
        tweaks: Vec<LeafKeyTweak>,
        receiver_public_key: &PublicKey,
    ) -> Result<Transfer, TransferServiceError> {
        todo!()
    }

    pub async fn query_transfer(
        &self,
        transfer_id: &str,
    ) -> Result<Option<Transfer>, TransferServiceError> {
        todo!()
    }

    pub async fn verify_pending_transfer(
        &self,
        transfer: &Transfer,
    ) -> Result<HashMap<String, PublicKey>, TransferServiceError> {
        todo!()
    }
}
