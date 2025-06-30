use std::collections::HashMap;
use std::time::Duration;

use crate::operator::rpc::{self as operator_rpc};
use bitcoin::{Transaction, secp256k1::PublicKey};
use uuid::Uuid;

use crate::{
    signer::Signer,
    tree::{TreeNode, TreeNodeId},
};

use super::ServiceError;

pub struct LeafKeyTweak {
    pub node: TreeNode,
    pub signing_public_key: PublicKey,
    pub new_signing_public_key: PublicKey,
}

pub struct Transfer {
    pub id: Uuid,
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

/// Configuration for claiming transfers
pub struct ClaimTransferConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub should_extend_timelocks: bool,
    pub should_refresh_timelocks: bool,
}

impl Default for ClaimTransferConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
            should_extend_timelocks: true,
            should_refresh_timelocks: true,
        }
    }
}

pub struct TransferService<S: Signer> {
    signer: S,
}

impl<S: Signer> TransferService<S> {
    pub fn new(signer: S) -> Self {
        Self { signer }
    }

    /// Claims a transfer with retry logic and automatic leaf preparation
    pub async fn claim_transfer(
        &self,
        transfer: &Transfer,
        config: Option<ClaimTransferConfig>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let config = config.unwrap_or_default();

        let mut retry_count = 0;
        loop {
            if retry_count >= config.max_retries {
                return Err(ServiceError::MaxRetriesExceeded);
            }

            // Introduce an exponential backoff delay before retrying.
            if retry_count > 0 {
                let delay_ms =
                    (config.base_delay_ms * 2u64.pow(retry_count - 1)).min(config.max_delay_ms);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            // Verify the pending transfer and get leaf pubkey map
            let leaf_pubkey_map = match self.verify_pending_transfer(transfer).await {
                Ok(map) => map,
                Err(_) => {
                    retry_count += 1;
                    continue;
                }
            };

            // Prepare leaves to claim
            let leaves_to_claim = match self
                .prepare_leaves_for_claiming(transfer, &leaf_pubkey_map)
                .await
            {
                Ok(leaves) => leaves,
                Err(ServiceError::NoLeavesToClaim) => {
                    return Ok(Vec::new());
                }
                Err(_) => {
                    retry_count += 1;
                    continue;
                }
            };

            // Actually claim the transfer
            let result = match self
                .claim_transfer_with_leaves(transfer, leaves_to_claim)
                .await
            {
                Ok(res) => res,
                Err(_) => {
                    retry_count += 1;
                    continue;
                }
            };

            // Post-process the claimed nodes
            let result = self.post_process_claimed_nodes(result, &config).await?;

            return Ok(result);
        }
    }

    /// Prepares leaves for claiming by creating LeafKeyTweak structs
    async fn prepare_leaves_for_claiming(
        &self,
        transfer: &Transfer,
        leaf_pubkey_map: &HashMap<TreeNodeId, PublicKey>,
    ) -> Result<Vec<LeafKeyTweak>, ServiceError> {
        let mut leaves_to_claim = Vec::new();

        for leaf in &transfer.leaves {
            let Some(leaf_pubkey) = leaf_pubkey_map.get(&leaf.leaf.id) else {
                continue;
            };
            leaves_to_claim.push(LeafKeyTweak {
                node: leaf.leaf.clone(),
                signing_public_key: *leaf_pubkey,
                new_signing_public_key: self.signer.get_public_key_for_node(&leaf.leaf.id)?,
            });
        }

        if leaves_to_claim.is_empty() {
            return Err(ServiceError::NoLeavesToClaim);
        }

        Ok(leaves_to_claim)
    }

    /// Post-processes claimed nodes (timelock operations)
    async fn post_process_claimed_nodes(
        &self,
        nodes: Vec<TreeNode>,
        config: &ClaimTransferConfig,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let mut result = nodes;

        if config.should_refresh_timelocks {
            result = self.check_refresh_timelock_nodes(result).await?;
        }

        if config.should_extend_timelocks {
            result = self.check_extend_timelock_nodes(result).await?;
        }

        Ok(result)
    }

    /// Checks and refreshes timelock nodes if needed
    async fn check_refresh_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: Implement timelock refresh logic
        // For now, return nodes unchanged
        Ok(nodes)
    }

    /// Checks and extends timelock nodes if needed
    async fn check_extend_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: Implement timelock extension logic
        // For now, return nodes unchanged
        Ok(nodes)
    }

    /// Low-level claim transfer operation
    async fn claim_transfer_with_leaves(
        &self,
        transfer: &Transfer,
        leaves_to_claim: Vec<LeafKeyTweak>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: implement according to js claimTransfer method
        todo!()
    }

    pub async fn extend_time_lock(&self, node: &TreeNode) -> Result<Vec<TreeNode>, ServiceError> {
        todo!()
    }

    pub async fn send_transfer_with_key_tweaks(
        &self,
        tweaks: &Vec<LeafKeyTweak>,
        receiver_public_key: &PublicKey,
    ) -> Result<Transfer, ServiceError> {
        todo!()
    }

    pub async fn query_pending_transfers(&self) -> Result<Vec<Transfer>, ServiceError> {
        todo!()
    }

    pub async fn transfer_leaves_to_self(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        todo!()
    }

    pub async fn query_transfer(
        &self,
        transfer_id: &Uuid,
    ) -> Result<Option<Transfer>, ServiceError> {
        todo!()
    }

    pub async fn verify_pending_transfer(
        &self,
        transfer: &Transfer,
    ) -> Result<HashMap<TreeNodeId, PublicKey>, ServiceError> {
        todo!()
    }

    pub async fn transfer_leaves_to(
        &self,
        leaves: &[TreeNode],
        receiver_id: &PublicKey,
    ) -> Result<Transfer, ServiceError> {
        let transfer_id = uuid::Uuid::now_v7();

        // build leaf key tweaks with new signing public key as a random key (for which we have the private key in memory only)
        // TODO: why?
        let leaf_key_tweaks = leaves
            .iter()
            .map(|leaf| {
                let signing_public_key = self.signer.get_public_key_for_node(&leaf.id)?;
                let new_signing_public_key = self.signer.generate_random_public_key()?;

                Ok(LeafKeyTweak {
                    node: leaf.clone(),
                    signing_public_key,
                    new_signing_public_key,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let leaves_tweaks_map: HashMap<String, Vec<operator_rpc::spark::SendLeafKeyTweak>> =
            HashMap::new();
        // TODO: build the map

        let transfer_package = operator_rpc::spark::TransferPackage {
            leaves_to_send: todo!(),
            key_tweak_package: todo!(), // built from the leaves_tweaks_map
            user_signature: todo!(),
        };

        // TODO: get spark client and make a start transfer request with the transfer package. Result contains transfer to return.

        todo!()
    }
}
