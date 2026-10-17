use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::secp256k1::{PublicKey, Secp256k1};
use spark::operator::OperatorPool;
use spark::operator::rpc::OperatorRpcError;
use spark::services::{
    ClaimTransferConfig, LeafKeyTweak, ServiceError, Transfer, TransferId, TransferService,
};
use spark::signer::{LeafSigningKey, SecretSource, Signer};
use spark::tree::{LeafPedigree, TreeNode, TreeNodeId, assemble_exit_chains, chain_reaches_root};

use crate::operator_rpc::{self, NodeQuery};
use crate::postgresql::PostgresLeafStore;
use crate::wakeup::Wakeup;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Which id derives a pool leaf's signing key. A deposit-tree leaf signs under the
/// leaf id the SSP picked, not the node id the operators assigned, until the SSP
/// claims the leaf from a transfer.
#[async_trait::async_trait]
pub trait LeafSigningKeys: Send + Sync {
    /// Returns, for each of `node_ids`, the id that the signer derives the
    /// node's signing key from. A node whose key the signer derives from the
    /// node's own id has no entry.
    async fn get_signing_leaf_ids(
        &self,
        node_ids: &[String],
    ) -> Result<HashMap<String, String>, String>;

    async fn mark_signing_under_node_id(&self, node_ids: &[String]) -> Result<(), String>;
}

pub async fn leaf_key_tweaks(
    signing_keys: &dyn LeafSigningKeys,
    leaves: &[TreeNode],
) -> Result<Vec<LeafKeyTweak>, String> {
    let node_ids: Vec<String> = leaves.iter().map(|leaf| leaf.id.to_string()).collect();
    let signing_ids = signing_keys.get_signing_leaf_ids(&node_ids).await?;
    leaves
        .iter()
        .map(|leaf| {
            let derived_from = match signing_ids.get(&leaf.id.to_string()) {
                Some(id) => id
                    .parse::<TreeNodeId>()
                    .map_err(|e| format!("invalid leaf id: {e}"))?,
                None => leaf.id.clone(),
            };
            Ok(LeafKeyTweak {
                node: leaf.clone(),
                signing_key: LeafSigningKey { derived_from },
            })
        })
        .collect()
}

pub struct IncomingTransfer {
    pub transfer: Transfer,
    held_under: HashMap<TreeNodeId, PublicKey>,
}

impl IncomingTransfer {
    pub async fn query(
        operator_pool: &OperatorPool,
        signer: &dyn Signer,
        network: spark::Network,
        transfer_id: &TransferId,
    ) -> Result<Option<Self>, ServiceError> {
        use spark::operator::rpc::spark::{TransferFilter, transfer_filter::Participant};

        let identity_public_key = spark::signer::derive_identity_public_key(signer).await?;
        let network: spark::operator::rpc::spark::Network = network.into();
        let response = operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                transfer_ids: vec![transfer_id.to_string()],
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    identity_public_key.serialize().to_vec(),
                )),
                network: network as i32,
                ..Default::default()
            })
            .await?;
        let Some(proto) = response.transfers.into_iter().next() else {
            return Ok(None);
        };
        let key = |bytes: &[u8]| PublicKey::from_slice(bytes).ok();
        let held_under = proto
            .leaves
            .iter()
            .filter_map(|leaf| {
                let node = leaf.leaf.as_ref()?;
                let held_under = held_under_after_tweak(
                    &key(&node.verifying_public_key)?,
                    &key(&node.signing_keyshare.as_ref()?.public_key)?,
                    &key(&leaf.pending_key_tweak_public_key)?,
                )?;
                Some((node.id.parse().ok()?, held_under))
            })
            .collect();
        Ok(Some(Self {
            transfer: Transfer::try_from(proto)?,
            held_under,
        }))
    }

    /// Whether every leaf's secret cipher is signed by the sender and holds the key
    /// the leaf is held under once the pending tweak applies. The operators cannot
    /// read the cipher, so they do not catch one holding another key.
    pub async fn is_claimable(
        &self,
        transfer_service: &TransferService,
        signer: &dyn Signer,
    ) -> bool {
        let Ok(secrets) = transfer_service
            .verify_pending_transfer(&self.transfer)
            .await
        else {
            return false;
        };
        for leaf in &self.transfer.leaves {
            let (Some(secret), Some(held_under)) = (
                secrets.get(&leaf.leaf.id),
                self.held_under.get(&leaf.leaf.id),
            ) else {
                return false;
            };
            let key = signer
                .public_key_from_secret(&SecretSource::Encrypted(secret.clone()))
                .await;
            if key.ok().as_ref() != Some(held_under) {
                return false;
            }
        }
        true
    }
}

/// The verifying key is the owner's key plus the operators' share. Applying
/// `tweak` (the old owner key minus the new one) adds it to the share, so the new
/// owner key is the verifying key minus the current share minus the tweak.
fn held_under_after_tweak(
    verifying: &PublicKey,
    operators: &PublicKey,
    tweak: &PublicKey,
) -> Option<PublicKey> {
    let secp = Secp256k1::verification_only();
    verifying
        .combine(&operators.negate(&secp))
        .and_then(|owner| owner.combine(&tweak.negate(&secp)))
        .ok()
}

#[derive(Debug, thiserror::Error)]
pub enum AncestorsError {
    #[error("could not query the coordinator: {0}")]
    Query(#[from] OperatorRpcError),

    #[error("the operators do not report the ancestors of leaf {0} up to its tree's root")]
    MissingAncestors(TreeNodeId),
}

pub async fn with_ancestors(
    operator_pool: &OperatorPool,
    network: spark::Network,
    leaves: &[TreeNode],
    query: NodeQuery,
) -> Result<Vec<LeafPedigree>, AncestorsError> {
    let ids: Vec<TreeNodeId> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
    let client = &operator_pool.get_coordinator().client;
    let mut nodes: HashMap<TreeNodeId, TreeNode> =
        operator_rpc::nodes(client, network, &ids, true, query)
            .await?
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
    // The caller's copy of a leaf has the caller's refund.
    nodes.extend(leaves.iter().map(|leaf| (leaf.id.clone(), leaf.clone())));
    let pedigrees = assemble_exit_chains(&nodes, &ids);
    if let Some(pedigree) = pedigrees
        .iter()
        .find(|pedigree| !chain_reaches_root(&pedigree.leaf, &pedigree.ancestors))
    {
        return Err(AncestorsError::MissingAncestors(pedigree.leaf.id.clone()));
    }
    Ok(pedigrees)
}

pub struct ClaimDeps {
    pub transfer_service: Arc<TransferService>,
    pub operator_pool: Arc<OperatorPool>,
    pub network: spark::Network,
    pub signing_keys: Arc<dyn LeafSigningKeys>,
    pub leaf_store: Arc<PostgresLeafStore>,
    pub renewal: Wakeup,
}

/// Once the SSP has claimed a leaf, the signer derives the leaf's signing key
/// from the leaf's node id. The function records that before it adds the
/// leaves to the pool: code that read the old id would sign with a key the
/// operators reject.
pub async fn claim_into_pool(
    deps: &ClaimDeps,
    transfer: &Transfer,
) -> Result<Vec<TreeNode>, BoxError> {
    let queried_at = chrono::Utc::now();
    // One attempt: retrying a failed claim is left to the caller.
    let claimed = deps
        .transfer_service
        .claim_transfer(
            transfer,
            Some(ClaimTransferConfig {
                max_retries: 1,
                ..ClaimTransferConfig::default()
            }),
        )
        .await?;
    let node_ids: Vec<String> = claimed.iter().map(|node| node.id.to_string()).collect();
    deps.signing_keys
        .mark_signing_under_node_id(&node_ids)
        .await?;
    let pedigrees = with_ancestors(
        &deps.operator_pool,
        deps.network,
        &claimed,
        NodeQuery::Public,
    )
    .await?;
    deps.leaf_store
        .claimed(&transfer.id, &pedigrees, queried_at)
        .await?;
    // The refund of a claimed leaf may need renewal before the SSP can use the
    // leaf.
    deps.renewal.wake();
    Ok(claimed)
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    use super::held_under_after_tweak;

    #[test]
    fn a_tweaked_leaf_is_held_under_the_new_key() {
        let secp = Secp256k1::new();
        let old = SecretKey::from_slice(&[1; 32]).unwrap();
        let operators = SecretKey::from_slice(&[2; 32]).unwrap();
        let new = SecretKey::from_slice(&[3; 32]).unwrap();
        let verifying = old
            .public_key(&secp)
            .combine(&operators.public_key(&secp))
            .unwrap();
        let tweak = old
            .public_key(&secp)
            .combine(&new.public_key(&secp).negate(&secp))
            .unwrap();

        assert_eq!(
            held_under_after_tweak(&verifying, &operators.public_key(&secp), &tweak),
            Some(new.public_key(&secp))
        );
    }
}
